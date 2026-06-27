//! Integration tests for the adbc extension's ADBC-facing logic, run directly against the Rust API
//! (the extension *protocol* is covered by the SDK's own `ext_vector` tests; the end-to-end Quoin
//! path is smoke-verified and deferred to an installation story). Every driver-backed test skips
//! cleanly when its ADBC driver isn't installed.
//!
//! Run: `cargo test --manifest-path crates/adbc/Cargo.toml`.

use super::*;

// ---- helpers -----------------------------------------------------------------------------------

/// A SQLite `:memory:` connection, or `None` if the SQLite ADBC driver isn't installed.
fn sqlite() -> Option<Connection> {
    Database::sqlite(":memory:")
        .ok()
        .and_then(|db| db.connect().ok())
}

/// A connection to the local PostgreSQL (passwordless `damon` via the `/tmp` socket), or `None` if
/// the server / driver isn't available.
fn postgres() -> Option<Connection> {
    Database::postgres("host=/tmp dbname=postgres user=damon")
        .ok()
        .and_then(|db| db.connect().ok())
}

/// Drain a query's `ResultSet` to the list of row `Map`s.
fn rows(rs: HandlerResult<ResultSet>) -> Vec<DataValue> {
    match rs.unwrap().drain().unwrap() {
        DataValue::List(v) => v,
        other => panic!("expected a list, got {other:?}"),
    }
}

/// One column of a row `Map`.
fn cell(row: &DataValue, name: &str) -> DataValue {
    match row {
        DataValue::Map(m) => m
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.clone())
            .unwrap_or_else(|| panic!("no column {name:?}")),
        other => panic!("expected a map, got {other:?}"),
    }
}

/// `select count(*)` of a table, as an `i64`.
fn count(c: &mut Connection, table: &str) -> i64 {
    let r = rows(c.query(&format!("select count(*) as n from {table}"), None));
    match cell(&r[0], "n") {
        DataValue::Int(n) => n,
        other => panic!("count not an int: {other:?}"),
    }
}

// ---- driver-free: parameter binding ------------------------------------------------------------

#[test]
fn param_binding_types() {
    let batch = bind_batch(&[
        DataValue::Int(1),
        DataValue::Float(2.5),
        DataValue::Str("x".into()),
        DataValue::Bool(true),
        DataValue::Bytes(vec![1, 2, 3]),
        DataValue::Null,
    ])
    .unwrap();
    assert_eq!(batch.num_rows(), 1);
    assert_eq!(batch.num_columns(), 6);
    assert_eq!(batch.column(0).data_type(), &DataType::Int64);
    assert_eq!(batch.column(1).data_type(), &DataType::Float64);
    assert_eq!(batch.column(2).data_type(), &DataType::Utf8);
    assert_eq!(batch.column(3).data_type(), &DataType::Boolean);
    assert_eq!(batch.column(4).data_type(), &DataType::Binary);
    // A Null binds as an Arrow `Null` column (the driver leaves the type unspecified).
    assert_eq!(batch.column(5).data_type(), &DataType::Null);
    // A List/Map isn't a scalar parameter.
    assert!(bind_batch(&[DataValue::List(vec![])]).is_err());
}

// ---- SQLite ------------------------------------------------------------------------------------

macro_rules! sqlite_test {
    ($name:ident, $c:ident, $body:block) => {
        #[test]
        fn $name() {
            let Some(mut $c) = sqlite() else {
                eprintln!(
                    "skipping {}: SQLite ADBC driver not installed",
                    stringify!($name)
                );
                return;
            };
            $body
        }
    };
}

sqlite_test!(sqlite_value_mapping, c, {
    let r = rows(c.query("select 1 as i, 3.5 as f, typeof(1) as t, null as z", None));
    assert_eq!(r.len(), 1);
    assert_eq!(cell(&r[0], "i"), DataValue::Int(1));
    assert_eq!(cell(&r[0], "f"), DataValue::Float(3.5));
    assert_eq!(cell(&r[0], "t"), DataValue::Str("integer".into()));
    assert_eq!(cell(&r[0], "z"), DataValue::Null);
});

sqlite_test!(sqlite_params_and_null, c, {
    c.execute("create table t (id integer, name text, age integer)", None)
        .unwrap();
    c.execute(
        "insert into t values (?, ?, ?)",
        Some(&[
            DataValue::Int(1),
            DataValue::Str("ada".into()),
            DataValue::Int(36),
        ]),
    )
    .unwrap();
    // A bound NULL (Arrow Null column) inserts and round-trips as nil.
    c.execute(
        "insert into t values (?, ?, ?)",
        Some(&[
            DataValue::Int(2),
            DataValue::Str("cy".into()),
            DataValue::Null,
        ]),
    )
    .unwrap();
    let null_age = rows(c.query("select name from t where age is null", None));
    assert_eq!(null_age.len(), 1);
    assert_eq!(cell(&null_age[0], "name"), DataValue::Str("cy".into()));
    // A bound param filters.
    let over = rows(c.query(
        "select id from t where age > ?",
        Some(&[DataValue::Int(18)]),
    ));
    assert_eq!(over.len(), 1);
    assert_eq!(cell(&over[0], "id"), DataValue::Int(1));
});

sqlite_test!(sqlite_streaming_and_schema, c, {
    // `next_row` yields one row at a time, then Null at end of stream.
    let mut rs = c
        .query("select 1 union select 2 union select 3", None)
        .unwrap();
    let mut n = 0;
    while let DataValue::Map(_) = rs.next_row().unwrap() {
        n += 1;
    }
    assert_eq!(n, 3);
    assert_eq!(rs.next_row().unwrap(), DataValue::Null);

    // `columns` (names) and the typed `schema` view.
    let rs2 = c.query("select 1 as n, 'x' as s", None).unwrap();
    assert_eq!(
        rs2.columns(),
        DataValue::List(vec![DataValue::Str("n".into()), DataValue::Str("s".into())])
    );
    let DataValue::List(cols) = rs2.schema_desc() else {
        panic!("schema not a list")
    };
    assert_eq!(cols.len(), 2);
    assert_eq!(cell(&cols[0], "name"), DataValue::Str("n".into()));
    assert_eq!(cell(&cols[0], "type"), DataValue::Str("Int64".into()));
    assert_eq!(cell(&cols[0], "nullable"), DataValue::Bool(true));
});

sqlite_test!(sqlite_prepared, c, {
    c.execute("create table t (id integer, name text)", None)
        .unwrap();
    c.execute(
        "insert into t values (1, ?)",
        Some(&[DataValue::Str("ada".into())]),
    )
    .unwrap();
    c.execute(
        "insert into t values (2, ?)",
        Some(&[DataValue::Str("bob".into())]),
    )
    .unwrap();
    let mut st = c.prepare("select name from t where id = ?").unwrap();
    st.bind(&[DataValue::Int(1)]).unwrap();
    assert_eq!(
        cell(&rows(st.query())[0], "name"),
        DataValue::Str("ada".into())
    );
    // Re-bind + re-query reuses the prepared statement.
    st.bind(&[DataValue::Int(2)]).unwrap();
    assert_eq!(
        cell(&rows(st.query())[0], "name"),
        DataValue::Str("bob".into())
    );
});

sqlite_test!(sqlite_transaction, c, {
    c.execute("create table t (a integer)", None).unwrap();
    c.set_autocommit(false).unwrap();
    c.execute("insert into t values (1)", None).unwrap();
    c.rollback().unwrap();
    assert_eq!(count(&mut c, "t"), 0, "rollback should discard the insert");
    c.execute("insert into t values (2)", None).unwrap();
    c.commit().unwrap();
    assert_eq!(count(&mut c, "t"), 1, "commit should keep the insert");
});

// ---- PostgreSQL (gated on the local server) ----------------------------------------------------

#[test]
fn postgres_params_null_and_types() {
    let Some(mut c) = postgres() else {
        eprintln!("skipping postgres_params_null_and_types: no local PostgreSQL / driver");
        return;
    };
    c.execute("drop table if exists _qn_adbc_test", None)
        .unwrap();
    c.execute(
        "create table _qn_adbc_test (id int, name text, ok boolean)",
        None,
    )
    .unwrap();
    c.execute(
        "insert into _qn_adbc_test values ($1, $2, $3)",
        Some(&[
            DataValue::Int(1),
            DataValue::Str("ada".into()),
            DataValue::Bool(true),
        ]),
    )
    .unwrap();
    // A bound NULL (the OID-0 path) inserts and round-trips.
    c.execute(
        "insert into _qn_adbc_test values ($1, $2, $3)",
        Some(&[DataValue::Int(2), DataValue::Null, DataValue::Bool(false)]),
    )
    .unwrap();
    let r = rows(c.query("select id, name, ok from _qn_adbc_test order by id", None));
    assert_eq!(r.len(), 2);
    assert_eq!(cell(&r[0], "ok"), DataValue::Bool(true));
    assert_eq!(cell(&r[1], "name"), DataValue::Null);
    assert_eq!(cell(&r[1], "ok"), DataValue::Bool(false));
    c.execute("drop table _qn_adbc_test", None).unwrap();
}
