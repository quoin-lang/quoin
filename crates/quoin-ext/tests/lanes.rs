//! Multi-lane serving, exercised from the host side of the wire: a served `Extension`
//! declaring `lanes(3)` accepts three connections, answers the manifest with the lane
//! count, and services calls on all three concurrently over the shared object table.

use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

use quoin_ext::{DataValue, Extension, read_frame, write_frame};
use quoin_ext_proto::{Arg, Msg, PROTOCOL_VERSION, decode_frame, encode};

struct Cell {
    n: i64,
}

/// How long each `slowDouble` handler blocks. Three serial calls would take three of
/// these; the overlap assertion allows two, so a scheduling hiccup can't flake it.
const SLEEP: Duration = Duration::from_millis(200);

fn sock_path(tag: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("quoin-ext-lanes-{tag}-{}.sock", std::process::id()))
}

fn connect_with_retry(path: &std::path::Path) -> UnixStream {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match UnixStream::connect(path) {
            Ok(s) => return s,
            Err(_) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(e) => panic!("connect {}: {e}", path.display()),
        }
    }
}

fn call(class: &str, op: &str, recv: u64, method_args: Vec<Arg>) -> Msg {
    Msg::Call {
        op: op.to_string(),
        arg: String::new(),
        handles: Vec::new(),
        resources: Vec::new(),
        releases: Vec::new(),
        arrays: Vec::new(),
        data: None,
        class_name: class.to_string(),
        recv,
        method_args,
    }
}

fn round_trip(stream: &mut UnixStream, msg: &Msg) -> Msg {
    write_frame(stream, &encode(msg)).unwrap();
    let frame = read_frame(stream).unwrap().expect("peer closed mid-call");
    decode_frame(&frame).unwrap()
}

#[test]
fn lanes_serve_concurrently_over_a_shared_table() {
    let path = sock_path("concurrent");
    let _ = std::fs::remove_file(&path);
    let server = std::thread::spawn({
        let path = path.clone();
        move || {
            let mut ext = Extension::new();
            ext.lanes(3);
            ext.class::<Cell>("Cell", |c| {
                c.constructor("of:", |_host, args| {
                    let Some(DataValue::Int(n)) = args[0].data() else {
                        return Err("of: expects an integer".into());
                    };
                    Ok(Cell { n: *n })
                });
                c.method("slowDouble", |cell, _host, _args| {
                    std::thread::sleep(SLEEP);
                    Ok(DataValue::Int(cell.n * 2))
                });
            });
            let _ = ext.serve(path.to_str().unwrap());
        }
    });

    // The handshake on the first connection declares the lane count.
    let mut conn0 = connect_with_retry(&path);
    match round_trip(
        &mut conn0,
        &Msg::GetManifest {
            version: PROTOCOL_VERSION,
        },
    ) {
        Msg::ManifestReturn { classes, lanes, .. } => {
            assert_eq!(lanes, 3);
            assert_eq!(classes.len(), 1);
            assert_eq!(classes[0].name, "Cell");
        }
        other => panic!("expected ManifestReturn, got {other:?}"),
    }

    // The two extra lanes connect after the manifest, as the host would.
    let conn1 = connect_with_retry(&path);
    let conn2 = connect_with_retry(&path);

    // Three instances built over lane 0; the table is shared, so any lane can serve them.
    let ids: Vec<u64> = (1..=3)
        .map(|i| {
            match round_trip(
                &mut conn0,
                &call("Cell", "of:", 0, vec![Arg::Data(DataValue::Int(i))]),
            ) {
                Msg::CallReturnResource { resource, .. } => resource,
                other => panic!("expected CallReturnResource, got {other:?}"),
            }
        })
        .collect();

    // One slow call per lane, to three different instances, all in flight at once.
    let started = Instant::now();
    let workers: Vec<_> = [conn0, conn1, conn2]
        .into_iter()
        .zip(ids)
        .enumerate()
        .map(|(i, (mut conn, id))| {
            std::thread::spawn(move || {
                match round_trip(&mut conn, &call("Cell", "slowDouble", id, Vec::new())) {
                    Msg::CallReturnData {
                        value: DataValue::Int(doubled),
                    } => assert_eq!(doubled, 2 * (i as i64 + 1)),
                    other => panic!("expected CallReturnData, got {other:?}"),
                }
                drop(conn);
            })
        })
        .collect();
    for w in workers {
        w.join().unwrap();
    }
    let elapsed = started.elapsed();
    assert!(elapsed >= SLEEP, "handlers must actually sleep");
    assert!(
        elapsed < SLEEP * 2,
        "three lane calls must overlap, took {elapsed:?} (serial would be {:?})",
        SLEEP * 3
    );

    server.join().unwrap();
    let _ = std::fs::remove_file(&path);
}

#[test]
fn single_lane_declares_one_and_serves_as_before() {
    let path = sock_path("single");
    let _ = std::fs::remove_file(&path);
    let server = std::thread::spawn({
        let path = path.clone();
        move || {
            let mut ext = Extension::new();
            ext.class::<Cell>("Cell", |c| {
                c.constructor("of:", |_host, args| {
                    let Some(DataValue::Int(n)) = args[0].data() else {
                        return Err("of: expects an integer".into());
                    };
                    Ok(Cell { n: *n })
                });
                c.method("double", |cell, _host, _args| {
                    Ok(DataValue::Int(cell.n * 2))
                });
            });
            let _ = ext.serve(path.to_str().unwrap());
        }
    });

    let mut conn = connect_with_retry(&path);
    match round_trip(
        &mut conn,
        &Msg::GetManifest {
            version: PROTOCOL_VERSION,
        },
    ) {
        Msg::ManifestReturn { lanes, .. } => assert_eq!(lanes, 1),
        other => panic!("expected ManifestReturn, got {other:?}"),
    }
    let id = match round_trip(
        &mut conn,
        &call("Cell", "of:", 0, vec![Arg::Data(DataValue::Int(21))]),
    ) {
        Msg::CallReturnResource { resource, .. } => resource,
        other => panic!("expected CallReturnResource, got {other:?}"),
    };
    match round_trip(&mut conn, &call("Cell", "double", id, Vec::new())) {
        Msg::CallReturnData {
            value: DataValue::Int(v),
        } => assert_eq!(v, 42),
        other => panic!("expected CallReturnData, got {other:?}"),
    }
    drop(conn);
    server.join().unwrap();
}
