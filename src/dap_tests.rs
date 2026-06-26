//! Tests for the DAP wire layer: `Content-Length` framing and the request/response/event
//! envelopes. The transport is pure (in-memory `Cursor`/`Vec`), so no VM is needed.

use super::*;
use std::io::Cursor;

#[test]
fn frame_round_trips_and_reports_eof() {
    let mut buf = Vec::new();
    write_frame(&mut buf, br#"{"hello":1}"#).unwrap();

    // Header is `Content-Length: <byte-len>\r\n\r\n` then the raw body.
    let text = String::from_utf8(buf.clone()).unwrap();
    assert_eq!(text, "Content-Length: 11\r\n\r\n{\"hello\":1}");

    let mut cur = Cursor::new(buf);
    let body = read_frame(&mut cur).unwrap().unwrap();
    assert_eq!(body, br#"{"hello":1}"#);
    // A clean EOF at a message boundary reads as `None`, not an error.
    assert!(read_frame(&mut cur).unwrap().is_none());
}

#[test]
fn reads_two_framed_messages_back_to_back() {
    let mut buf = Vec::new();
    write_frame(&mut buf, br#"{"seq":1}"#).unwrap();
    write_frame(&mut buf, br#"{"seq":2}"#).unwrap();
    let mut cur = Cursor::new(buf);
    assert_eq!(read_frame(&mut cur).unwrap().unwrap(), br#"{"seq":1}"#);
    assert_eq!(read_frame(&mut cur).unwrap().unwrap(), br#"{"seq":2}"#);
    assert!(read_frame(&mut cur).unwrap().is_none());
}

#[test]
fn parses_request_and_serializes_response_and_event() {
    let mut framed = Vec::new();
    write_frame(
        &mut framed,
        br#"{"seq":3,"type":"request","command":"initialize","arguments":{"adapterID":"quoin"}}"#,
    )
    .unwrap();

    let mut conn = Connection::new(Cursor::new(framed), Vec::new());
    let req = conn.read_request().unwrap().unwrap();
    assert_eq!(req.seq, 3);
    assert_eq!(req.command, "initialize");
    assert_eq!(req.arguments["adapterID"], "quoin");

    conn.ok(
        &req,
        Some(serde_json::json!({ "supportsConfigurationDoneRequest": true })),
    )
    .unwrap();
    conn.event("initialized", None).unwrap();

    let out = String::from_utf8(conn.into_writer()).unwrap();
    // The response echoes the request seq/command and reports success...
    assert!(out.contains(r#""type":"response""#), "{out}");
    assert!(out.contains(r#""request_seq":3"#), "{out}");
    assert!(out.contains(r#""command":"initialize""#), "{out}");
    assert!(out.contains(r#""success":true"#), "{out}");
    assert!(
        out.contains(r#""supportsConfigurationDoneRequest":true"#),
        "{out}"
    );
    // ...and the event carries its name. Both are Content-Length framed.
    assert!(out.contains(r#""type":"event""#), "{out}");
    assert!(out.contains(r#""event":"initialized""#), "{out}");
    assert_eq!(out.matches("Content-Length:").count(), 2, "{out}");
}

#[test]
fn request_arguments_default_to_null_when_absent() {
    let mut framed = Vec::new();
    write_frame(
        &mut framed,
        br#"{"seq":7,"type":"request","command":"threads"}"#,
    )
    .unwrap();
    let mut conn = Connection::new(Cursor::new(framed), Vec::new());
    let req = conn.read_request().unwrap().unwrap();
    assert_eq!(req.command, "threads");
    assert!(req.arguments.is_null());
}

#[test]
fn failed_response_carries_message_and_omits_body() {
    let mut framed = Vec::new();
    write_frame(
        &mut framed,
        br#"{"seq":1,"type":"request","command":"launch"}"#,
    )
    .unwrap();
    let mut conn = Connection::new(Cursor::new(framed), Vec::new());
    let req = conn.read_request().unwrap().unwrap();
    conn.fail(&req, "no such file").unwrap();

    let out = String::from_utf8(conn.into_writer()).unwrap();
    assert!(out.contains(r#""success":false"#), "{out}");
    assert!(out.contains(r#""message":"no such file""#), "{out}");
    // `body` is skipped when None.
    assert!(!out.contains(r#""body""#), "{out}");
}
