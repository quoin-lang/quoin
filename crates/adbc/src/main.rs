//! `adbc` — Quoin's ADBC database extension (out-of-process, out-of-core). See `DESIGN.md`.
//!
//! Slice 1: the crate skeleton, driver loading (via the ADBC driver-manifest), and the start of the
//! class chain — `[ADBC]Database` (open SQLite / PostgreSQL) → `connect` → `[ADBC]Connection` with a
//! first `query:` that materializes rows as a `List` of `Map`s. (Streaming `ResultSet`, DML/params,
//! transactions, and metadata are later slices.) Every ADBC fallibility threads through the SDK's
//! `HandlerResult`, so a driver-load failure or a SQL error surfaces as a *catchable* Quoin error
//! and the extension stays alive.

use std::path::PathBuf;

use adbc_core::options::{AdbcVersion, OptionDatabase, OptionValue};
use adbc_core::sync::{Connection as _, Database as _, Driver as _, Statement as _};
use adbc_driver_manager::ManagedDriver;
use arrow_array::{Array, RecordBatch};
use arrow_schema::DataType;
use quoin_ext::{DataValue, Extension, HandlerResult};

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
    /// Run a SQL query and materialize every row as a `Map` (column -> value). (Slice 2 makes this a
    /// streaming `ResultSet`; this eager form is the round-trip proof.) A SQL error surfaces as a
    /// catchable Quoin error — the connection stays alive.
    fn query(&mut self, sql: &str) -> HandlerResult<DataValue> {
        let mut stmt = self.conn.new_statement()?;
        stmt.set_sql_query(sql)?;
        let reader = stmt.execute()?;
        let mut rows = Vec::new();
        for batch in reader {
            rows.extend(batch_rows(&batch?));
        }
        Ok(DataValue::List(rows))
    }
}

// ---- Arrow -> DataValue --------------------------------------------------------------------------

/// Every row of `batch` as a `Map` (column name -> cell value).
fn batch_rows(batch: &RecordBatch) -> Vec<DataValue> {
    let names: Vec<String> = batch
        .schema()
        .fields()
        .iter()
        .map(|f| f.name().clone())
        .collect();
    (0..batch.num_rows())
        .map(|r| {
            let cells = names
                .iter()
                .enumerate()
                .map(|(c, name)| (name.clone(), cell_value(batch.column(c), r)))
                .collect();
            DataValue::Map(cells)
        })
        .collect()
}

/// One Arrow cell -> a `DataValue` (slice-1 subset; the fuller mapping table lands with the
/// streaming `ResultSet`). Unknown types fall back to a debug string, never silently wrong.
fn cell_value(col: &dyn Array, row: usize) -> DataValue {
    use arrow_array::cast::AsArray;
    use arrow_array::types::{Float64Type, Int32Type, Int64Type};
    if col.is_null(row) {
        return DataValue::Null;
    }
    match col.data_type() {
        DataType::Int64 => DataValue::Int(col.as_primitive::<Int64Type>().value(row)),
        DataType::Int32 => DataValue::Int(col.as_primitive::<Int32Type>().value(row) as i64),
        DataType::Float64 => DataValue::Float(col.as_primitive::<Float64Type>().value(row)),
        DataType::Utf8 => DataValue::Str(col.as_string::<i32>().value(row).to_string()),
        DataType::LargeUtf8 => DataValue::Str(col.as_string::<i64>().value(row).to_string()),
        DataType::Boolean => DataValue::Bool(col.as_boolean().value(row)),
        _ => DataValue::Str(format!("{:?}", col.data_type())),
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
        c.method("query:", |conn, _h, args| conn.query(str_arg(args, 0)));
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
