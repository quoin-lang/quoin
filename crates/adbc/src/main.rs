//! `adbc` — Quoin's ADBC database extension (out-of-process, out-of-core). See `DESIGN.md`.
//!
//! The class chain so far: `[ADBC]Database` (open SQLite / PostgreSQL) → `connect` →
//! `[ADBC]Connection` → `query:` → a streaming `[ADBC]ResultSet` (`next` / `each:` / `toList` /
//! `columns` / `schema` / `close`), with the Arrow→DataValue value mapping. (DML / params /
//! `prepare:`, transactions, and metadata are later slices.) Every ADBC fallibility threads through
//! the SDK's `HandlerResult`, so a driver-load failure or a SQL error surfaces as a *catchable*
//! Quoin error and the extension stays alive.

use std::path::PathBuf;

use adbc_core::options::{AdbcVersion, OptionDatabase, OptionValue};
use adbc_core::sync::{Connection as _, Database as _, Driver as _, Statement as _};
use adbc_driver_manager::ManagedDriver;
use arrow_array::{Array, RecordBatch, RecordBatchReader};
use arrow_cast::display::array_value_to_string;
use arrow_schema::{DataType, SchemaRef};
use quoin_ext::{DataValue, Extension, Handle, HandlerResult, Host};

// ---- driver resolution (ADBC driver-manifest) --------------------------------------------------

/// The current platform's key in an ADBC driver manifest's `[Driver.shared]` table (e.g. `macos_arm64`).
fn platform_key() -> String {
    let os = if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "unknown"
    };
    let arch = if cfg!(target_arch = "aarch64") {
        "arm64"
    } else if cfg!(target_arch = "x86_64") {
        "amd64"
    } else {
        "unknown"
    };
    format!("{os}_{arch}")
}

/// Directories searched for ADBC driver manifests (`<name>.toml`), platform-appropriate.
fn manifest_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        dirs.push(home.join("Library/Application Support/ADBC/Drivers")); // macOS
        dirs.push(home.join(".local/share/adbc/drivers")); // Linux (XDG)
    }
    dirs.push(PathBuf::from("/usr/local/etc/adbc/drivers"));
    dirs.push(PathBuf::from("/etc/adbc/drivers"));
    dirs
}

/// Resolve a driver name (`sqlite` / `postgresql`) to its shared-library path: an env override
/// (`QUOIN_ADBC_<NAME>_PATH`) wins; otherwise read the driver manifest `<name>.toml` and pull the
/// `[Driver.shared].<platform>` entry. (adbc_driver_manager 0.23 does not resolve manifests itself.)
fn resolve_driver_path(name: &str) -> Result<PathBuf, String> {
    let env_key = format!("QUOIN_ADBC_{}_PATH", name.to_uppercase());
    if let Some(p) = std::env::var_os(&env_key) {
        return Ok(PathBuf::from(p));
    }
    let key = platform_key();
    for dir in manifest_dirs() {
        let manifest = dir.join(format!("{name}.toml"));
        let Ok(text) = std::fs::read_to_string(&manifest) else {
            continue;
        };
        if let Some(path) = manifest_shared_path(&text, &key) {
            return Ok(PathBuf::from(path));
        }
    }
    Err(format!(
        "no ADBC driver '{name}' found (set {env_key}, or install a manifest for platform '{key}')"
    ))
}

/// Pull `[Driver.shared].<key> = '<path>'` out of a driver manifest. A deliberately small reader of
/// the fixed ADBC-manifest shape (a real TOML parse is a later refinement once the manifest grows).
fn manifest_shared_path(text: &str, key: &str) -> Option<String> {
    let prefix = format!("{key} =");
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix(&prefix) {
            let v = rest.trim().trim_matches(['\'', '"']);
            return Some(v.to_string());
        }
    }
    None
}

/// Load an ADBC driver by name (resolving its manifest) at ADBC 1.1.0, default entrypoint.
fn load_driver(name: &str) -> HandlerResult<ManagedDriver> {
    let path = resolve_driver_path(name)?;
    Ok(ManagedDriver::load_dynamic_from_filename(
        &path,
        None,
        AdbcVersion::V110,
    )?)
}

// ---- the database handles (live in the SDK object table) ---------------------------------------

/// An open data source. Keeps its driver alive alongside the database handle.
struct Database {
    _driver: ManagedDriver,
    db: adbc_driver_manager::ManagedDatabase,
}

impl Database {
    /// Open `name` (a driver) with the database `uri`.
    fn open(name: &str, uri: &str) -> HandlerResult<Database> {
        let mut driver = load_driver(name)?;
        let db = driver.new_database_with_opts([(
            OptionDatabase::Uri,
            OptionValue::String(uri.to_string()),
        )])?;
        Ok(Database {
            _driver: driver,
            db,
        })
    }

    fn sqlite(uri: &str) -> HandlerResult<Database> {
        Database::open("sqlite", uri)
    }

    fn postgres(conn: &str) -> HandlerResult<Database> {
        Database::open("postgresql", conn)
    }

    /// Open a new session. (A `Connection` holds the database alive via ADBC's Arc chain.)
    fn connect(&self) -> HandlerResult<Connection> {
        Ok(Connection {
            conn: self.db.new_connection()?,
        })
    }
}

/// A live session.
struct Connection {
    conn: adbc_driver_manager::ManagedConnection,
}

impl Connection {
    /// Run a SQL query, returning a streaming [`ResultSet`]. The `Statement` moves *into* the result:
    /// despite the `'static` bound on `execute()`'s reader, some drivers (PostgreSQL/libpq) invalidate
    /// the reader once its statement closes, so the `ResultSet` must keep the statement alive. A SQL
    /// error surfaces as a catchable Quoin error — the connection stays alive.
    fn query(&mut self, sql: &str) -> HandlerResult<ResultSet> {
        let mut stmt = self.conn.new_statement()?;
        stmt.set_sql_query(sql)?;
        let reader = stmt.execute()?;
        Ok(ResultSet::new(stmt, reader))
    }
}

/// A streaming query result (`[ADBC]ResultSet`): the live Arrow reader plus a one-batch buffer,
/// walked row-by-row. Held in the SDK object table; a new batch is pulled only when the current one
/// is exhausted, so a large result never fully materializes host-side.
struct ResultSet {
    // Field order is the drop order: the reader is released *before* the statement it reads from,
    // because some drivers (PostgreSQL) invalidate the reader when its statement closes.
    reader: Option<Box<dyn RecordBatchReader + Send>>,
    batch: Option<RecordBatch>,
    schema: SchemaRef,
    row: usize,
    // Kept alive solely so the reader stays valid (and, via its Arc chain, the connection).
    stmt: Option<adbc_driver_manager::ManagedStatement>,
}

impl ResultSet {
    fn new(
        stmt: adbc_driver_manager::ManagedStatement,
        reader: Box<dyn RecordBatchReader + Send>,
    ) -> ResultSet {
        let schema = reader.schema();
        ResultSet {
            reader: Some(reader),
            batch: None,
            schema,
            row: 0,
            stmt: Some(stmt),
        }
    }

    /// The next row as a `Map` (column -> value), or `Null` at end of stream. Lazy: a fresh batch is
    /// fetched only when the buffered one runs out.
    fn next_row(&mut self) -> HandlerResult<DataValue> {
        loop {
            if let Some(batch) = &self.batch
                && self.row < batch.num_rows()
            {
                let m = row_map(batch, self.row);
                self.row += 1;
                return Ok(m);
            }
            let Some(reader) = self.reader.as_mut() else {
                return Ok(DataValue::Null);
            };
            match reader.next() {
                Some(batch) => {
                    self.batch = Some(batch?);
                    self.row = 0;
                }
                None => {
                    self.close();
                    return Ok(DataValue::Null);
                }
            }
        }
    }

    /// Drain every remaining row into a `List` of `Map`s.
    fn drain(&mut self) -> HandlerResult<DataValue> {
        let mut rows = Vec::new();
        while let row @ DataValue::Map(_) = self.next_row()? {
            rows.push(row);
        }
        Ok(DataValue::List(rows))
    }

    /// Apply `block` to each remaining row `Map` (one batched round-trip per row).
    fn each(&mut self, host: &mut Host, block: Handle) -> HandlerResult<DataValue> {
        while let row @ DataValue::Map(_) = self.next_row()? {
            host.apply_block(block, &[row])?;
        }
        Ok(DataValue::Null)
    }

    /// Column names — the cheap "what columns does this have" case.
    fn columns(&self) -> DataValue {
        DataValue::List(
            self.schema
                .fields()
                .iter()
                .map(|f| DataValue::Str(f.name().clone()))
                .collect(),
        )
    }

    /// The typed column view: one `Map` per column with `name` / `type` / `nullable` (the `type` is
    /// Arrow's canonical name, e.g. `Int64`, `Utf8`, `Timestamp(Microsecond, "UTC")`).
    fn schema_desc(&self) -> DataValue {
        DataValue::List(
            self.schema
                .fields()
                .iter()
                .map(|f| {
                    DataValue::Map(vec![
                        ("name".to_string(), DataValue::Str(f.name().clone())),
                        (
                            "type".to_string(),
                            DataValue::Str(format!("{}", f.data_type())),
                        ),
                        ("nullable".to_string(), DataValue::Bool(f.is_nullable())),
                    ])
                })
                .collect(),
        )
    }

    /// Release the reader (and its cursor) now, rather than waiting for the table to reap it. The
    /// reader is dropped before the statement it read from.
    fn close(&mut self) {
        self.reader = None;
        self.batch = None;
        self.stmt = None;
    }
}

// ---- Arrow -> DataValue --------------------------------------------------------------------------

/// One row of `batch` as a `Map` (column name -> cell value).
fn row_map(batch: &RecordBatch, row: usize) -> DataValue {
    let cells = batch
        .schema()
        .fields()
        .iter()
        .enumerate()
        .map(|(c, f)| (f.name().clone(), cell_value(batch.column(c), row)))
        .collect();
    DataValue::Map(cells)
}

/// Format one cell the way Arrow's own pretty-printer would (ISO-8601 for temporals, a readable
/// rendering for decimals / nested types). Falls back to the type name if formatting fails.
fn formatted(col: &dyn Array, row: usize) -> String {
    array_value_to_string(col, row).unwrap_or_else(|_| format!("{}", col.data_type()))
}

/// One Arrow cell -> a `DataValue` (DESIGN §5). Numbers / strings / bytes map natively; decimals
/// carry their decimal-string form; temporals become ISO-8601 strings; anything else (list / struct
/// / views) falls back to its formatted value — never silently wrong.
fn cell_value(col: &dyn Array, row: usize) -> DataValue {
    use arrow_array::cast::AsArray;
    use arrow_array::types::{
        Float16Type, Float32Type, Float64Type, Int8Type, Int16Type, Int32Type, Int64Type,
        UInt8Type, UInt16Type, UInt32Type, UInt64Type,
    };
    if col.is_null(row) {
        return DataValue::Null;
    }
    match col.data_type() {
        DataType::Boolean => DataValue::Bool(col.as_boolean().value(row)),
        DataType::Int8 => DataValue::Int(col.as_primitive::<Int8Type>().value(row) as i64),
        DataType::Int16 => DataValue::Int(col.as_primitive::<Int16Type>().value(row) as i64),
        DataType::Int32 => DataValue::Int(col.as_primitive::<Int32Type>().value(row) as i64),
        DataType::Int64 => DataValue::Int(col.as_primitive::<Int64Type>().value(row)),
        DataType::UInt8 => DataValue::Int(col.as_primitive::<UInt8Type>().value(row) as i64),
        DataType::UInt16 => DataValue::Int(col.as_primitive::<UInt16Type>().value(row) as i64),
        DataType::UInt32 => DataValue::Int(col.as_primitive::<UInt32Type>().value(row) as i64),
        DataType::UInt64 => {
            let v = col.as_primitive::<UInt64Type>().value(row);
            i64::try_from(v).map_or_else(|_| DataValue::BigInt(v.to_string()), DataValue::Int)
        }
        DataType::Float16 => {
            DataValue::Float(f64::from(col.as_primitive::<Float16Type>().value(row)))
        }
        DataType::Float32 => DataValue::Float(col.as_primitive::<Float32Type>().value(row) as f64),
        DataType::Float64 => DataValue::Float(col.as_primitive::<Float64Type>().value(row)),
        DataType::Utf8 => DataValue::Str(col.as_string::<i32>().value(row).to_string()),
        DataType::LargeUtf8 => DataValue::Str(col.as_string::<i64>().value(row).to_string()),
        DataType::Binary => DataValue::Bytes(col.as_binary::<i32>().value(row).to_vec()),
        DataType::LargeBinary => DataValue::Bytes(col.as_binary::<i64>().value(row).to_vec()),
        DataType::Decimal128(_, _) | DataType::Decimal256(_, _) => {
            DataValue::Decimal(formatted(col, row))
        }
        DataType::Date32
        | DataType::Date64
        | DataType::Timestamp(_, _)
        | DataType::Time32(_)
        | DataType::Time64(_) => DataValue::Str(formatted(col, row)),
        _ => DataValue::Str(formatted(col, row)),
    }
}

// ---- the extension -----------------------------------------------------------------------------

fn main() {
    let path = std::env::args().nth(1).expect("usage: adbc <socket-path>");

    let mut ext = Extension::new();
    ext.class::<Database>("[ADBC]Database", |c| {
        c.constructor("sqlite:", |_h, args| Database::sqlite(str_arg(args, 0)));
        c.constructor("sqliteMemory", |_h, _a| Database::sqlite(":memory:"));
        c.constructor("postgres:", |_h, args| Database::postgres(str_arg(args, 0)));
        c.constructor("driver:uri:", |_h, args| {
            Database::open(str_arg(args, 0), str_arg(args, 1))
        });
        c.makes("connect", |db, _h, _a| db.connect());
    });
    ext.class::<Connection>("[ADBC]Connection", |c| {
        c.makes("query:", |conn, _h, args| conn.query(str_arg(args, 0)));
    });
    ext.class::<ResultSet>("[ADBC]ResultSet", |c| {
        c.method("next", |rs, _h, _a| rs.next_row());
        c.method("toList", |rs, _h, _a| rs.drain());
        c.method("columns", |rs, _h, _a| Ok(rs.columns()));
        c.method("schema", |rs, _h, _a| Ok(rs.schema_desc()));
        c.method("each:", |rs, host, args| {
            let block = args[0].handle().ok_or("each: expects a block")?;
            rs.each(host, block)
        });
        c.method("close", |rs, _h, _a| {
            rs.close();
            Ok(DataValue::Null)
        });
    });
    ext.serve(&path).expect("adbc serve loop");
}

/// Read the `n`th argument as a string (the SDK delivers method args as `DataValue`s).
fn str_arg<'a>(args: &'a [quoin_ext::Arg], n: usize) -> &'a str {
    match args.get(n).and_then(|a| a.data()) {
        Some(DataValue::Str(s)) => s,
        _ => "",
    }
}
