use crate::{
    Arg, ArrowArray, ArrowDType, ClassDecl, DataValue as Dv, Msg, decode_frame, encode, pack_dv,
    unpack_dv,
};

fn round_trip_msg(msg: Msg) {
    let frame = encode(&msg);
    assert_eq!(
        decode_frame(&frame).unwrap(),
        msg,
        "frame round trip failed"
    );
}

fn round_trip(dv: Dv) {
    assert_eq!(unpack_dv(&pack_dv(&dv)).unwrap(), dv, "round trip failed");
}

#[test]
fn every_message_round_trips() {
    let arrow = ArrowArray {
        dtype: ArrowDType::Int64,
        length: 2,
        data: vec![1, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0],
    };
    let msgs = vec![
        Msg::Call {
            op: "echo".into(),
            arg: "x".into(),
            handles: vec![1, 2],
            resources: vec![3],
            releases: vec![4, 5, 6],
            arrays: vec![arrow.clone()],
            data: Some(Dv::Map(vec![
                ("k".into(), Dv::Int(1)),
                ("v".into(), Dv::List(vec![Dv::Float(1.5), Dv::Null])),
            ])),
            class_name: "Array".into(),
            recv: 7,
            method_args: vec![
                Arg::Data(Dv::Str("s".into())),
                Arg::Resource(9),
                Arg::Handle(u64::MAX),
            ],
        },
        Msg::Call {
            op: "bare".into(),
            arg: String::new(),
            handles: vec![],
            resources: vec![],
            releases: vec![],
            arrays: vec![],
            data: None,
            class_name: String::new(),
            recv: 0,
            method_args: vec![],
        },
        Msg::CallReturn { result: "r".into() },
        Msg::CallReturnError {
            message: "boom".into(),
        },
        Msg::CallReturnResource {
            resource: 42,
            class_name: "Vector".into(),
        },
        Msg::CallReturnArray {
            array: arrow.clone(),
        },
        Msg::CallReturnData {
            value: Dv::List(vec![Dv::Bool(true), Dv::Bytes(vec![0, 255])]),
        },
        Msg::CallReturnHandle { handle: 11 },
        Msg::GetManifest { version: 2 },
        Msg::ManifestReturn {
            classes: vec![ClassDecl {
                name: "Array".into(),
                instance_selectors: vec!["at:".into(), "sum".into()],
                class_selectors: vec!["zeros:".into()],
            }],
            version: 2,
        },
        Msg::MakeString { value: "s".into() },
        Msg::HandleToString { handle: 3 },
        Msg::Retain { handle: 4 },
        Msg::Release {
            handles: vec![5, 6],
        },
        Msg::CallMethodOnHandle {
            receiver: 1,
            selector: "at:put:".into(),
            args: vec![2, 3],
        },
        Msg::InvokeBlock {
            block: 8,
            batches: vec![vec![1, 2], vec![], vec![3]],
        },
        Msg::InvokeBlockReturn {
            results: vec![10, 11, 12],
            error: None,
        },
        Msg::InvokeBlockReturn {
            results: vec![],
            error: Some("bad".into()),
        },
        Msg::GetGlobal {
            name: "Timer".into(),
        },
        Msg::MakeValue {
            value: Dv::BigInt("123456789012345678901234567890".into()),
        },
        Msg::ReadHandle { handle: 9 },
        Msg::ReadHandleReturn {
            value: Dv::Decimal("-1.500".into()),
            error: None,
        },
        Msg::ReadHandleReturn {
            value: Dv::Null,
            error: Some("no such handle".into()),
        },
        Msg::HostOpReturn {
            handle: 13,
            str: Some("s".into()),
            error: None,
        },
        Msg::HostOpReturn {
            handle: 0,
            str: None,
            error: Some("nope".into()),
        },
    ];
    for msg in msgs {
        round_trip_msg(msg);
    }
}

#[test]
fn call_data_some_null_collapses_to_none() {
    // `Some(Null)` and `None` are indistinguishable on every SDK surface, so the wire
    // collapses them: both encode as nil, which decodes as `None`.
    let frame = encode(&Msg::Call {
        op: "x".into(),
        arg: String::new(),
        handles: vec![],
        resources: vec![],
        releases: vec![],
        arrays: vec![],
        data: Some(Dv::Null),
        class_name: String::new(),
        recv: 0,
        method_args: vec![],
    });
    match decode_frame(&frame).unwrap() {
        Msg::Call { data, .. } => assert_eq!(data, None),
        other => panic!("unexpected msg: {other:?}"),
    }
}

#[test]
fn extra_trailing_fields_are_skipped() {
    // Append-only evolution: a newer peer may add fields; this decoder must skip them.
    // Hand-build `[T_CALL_RETURN, "x", nil, [1, {"k": 2}]]` (two unknown extras).
    let frame = vec![
        0x94, // array of 4
        0x01, // T_CALL_RETURN
        0xa1, b'x', // "x"
        0xc0, // extra: nil
        0x92, 0x01, 0x81, 0xa1, b'k', 0x02, // extra: [1, {"k": 2}]
    ];
    match decode_frame(&frame).unwrap() {
        Msg::CallReturn { result } => assert_eq!(result, "x"),
        other => panic!("unexpected msg: {other:?}"),
    }
}

#[test]
fn unknown_frame_type_is_a_clear_error() {
    let frame = vec![0x91, 0x63]; // [99]
    let err = decode_frame(&frame).expect_err("unknown type must be rejected");
    assert!(
        err.contains("unknown frame type 99"),
        "unexpected error: {err}"
    );
    assert!(err.contains("protocol version"), "unexpected error: {err}");
}

#[test]
fn too_few_fields_is_a_clear_error() {
    let frame = vec![0x91, 0x01]; // [T_CALL_RETURN] with no result field
    let err = decode_frame(&frame).expect_err("short frame must be rejected");
    assert!(err.contains("CallReturn"), "unexpected error: {err}");
}

#[test]
fn truncated_and_trailing_frames_are_rejected() {
    let mut frame = encode(&Msg::CallReturn {
        result: "xyz".into(),
    });
    let full = frame.clone();
    frame.truncate(frame.len() - 1);
    assert!(decode_frame(&frame).is_err(), "truncated frame accepted");
    let mut trailing = full;
    trailing.push(0xc0);
    assert!(decode_frame(&trailing).is_err(), "trailing bytes accepted");
}

#[test]
fn deep_datavalue_is_rejected_not_overflowed() {
    // A value nested well past the cap must return an error, never overflow the (host)
    // stack — and the same must hold on the skip path (extras in a newer-peer frame).
    let mut dv = Dv::Int(1);
    for _ in 0..300 {
        dv = Dv::List(vec![dv]);
    }
    let frame = encode(&Msg::CallReturnData { value: dv.clone() });
    let err = decode_frame(&frame).expect_err("deep value must be rejected");
    assert!(err.contains("nesting"), "unexpected error: {err}");

    // Same nest smuggled in as an unknown extra field: `[T_CALL_RETURN, "x", <deep>]`.
    let mut frame = vec![0x93, 0x01, 0xa1, b'x'];
    frame.extend_from_slice(&pack_dv(&dv));
    let err = decode_frame(&frame).expect_err("deep extra must be rejected");
    assert!(err.contains("nesting"), "unexpected error: {err}");
}

#[test]
fn scalars_round_trip() {
    for dv in [
        Dv::Null,
        Dv::Bool(true),
        Dv::Bool(false),
        Dv::Int(0),
        Dv::Int(127),
        Dv::Int(128),
        Dv::Int(-1),
        Dv::Int(-32),
        Dv::Int(-33),
        Dv::Int(65536),
        Dv::Int(i64::MAX),
        Dv::Int(i64::MIN),
        Dv::Float(1.5),
        Dv::Str(String::new()),
        Dv::Str("hello".into()),
        Dv::Str("x".repeat(40)),
        Dv::Str("y".repeat(70000)),
        Dv::Bytes(vec![]),
        Dv::Bytes(vec![0, 255, 7]),
        Dv::Bytes(vec![9; 70000]),
        Dv::BigInt("123456789012345678901234567890".into()),
        Dv::Decimal("-1.500".into()),
    ] {
        round_trip(dv);
    }
    // NaN != NaN; check the bits instead.
    let f = f64::NAN.copysign(1.0);
    match unpack_dv(&pack_dv(&Dv::Float(f))).unwrap() {
        Dv::Float(g) => assert_eq!(f.to_bits(), g.to_bits()),
        other => panic!("unexpected: {other:?}"),
    }
}

#[test]
fn structures_round_trip() {
    round_trip(Dv::List(vec![]));
    round_trip(Dv::List((0..300).map(Dv::Int).collect()));
    round_trip(Dv::Map(vec![
        ("a".into(), Dv::Int(1)),
        (
            "nested".into(),
            Dv::List(vec![Dv::Str("x".into()), Dv::Null]),
        ),
    ]));
    round_trip(Dv::Map(
        (0..40)
            .map(|i| (format!("k{i}"), Dv::Float(i as f64)))
            .collect(),
    ));
}

#[test]
fn depth_cap_rejects_deep_values() {
    let mut dv = Dv::Int(1);
    for _ in 0..300 {
        dv = Dv::List(vec![dv]);
    }
    let err = unpack_dv(&pack_dv(&dv)).expect_err("deep packed value must be rejected");
    assert!(err.contains("nesting"), "unexpected error: {err}");
}

#[test]
fn trailing_garbage_rejected() {
    let mut b = pack_dv(&Dv::Int(1));
    b.push(0xc0);
    assert!(unpack_dv(&b).is_err());
}

#[test]
fn uint64_beyond_i64_becomes_bigint() {
    // 0xcf marker with a value above i64::MAX (a foreign packer may emit this).
    let mut b = vec![0xcf];
    b.extend_from_slice(&u64::MAX.to_be_bytes());
    assert_eq!(unpack_dv(&b).unwrap(), Dv::BigInt(u64::MAX.to_string()));
}

#[test]
fn float32_from_foreign_packer_decodes() {
    // Our writer never emits 0xca, but a foreign packer may.
    let mut b = vec![0xca];
    b.extend_from_slice(&1.5f32.to_be_bytes());
    assert_eq!(unpack_dv(&b).unwrap(), Dv::Float(1.5));
}
