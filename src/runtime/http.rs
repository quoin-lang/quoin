use crate::arg;
use crate::error::QuoinError;
use crate::value::{NativeClassBuilder, Value};

/// Max headers we'll parse in a head (either direction). A head with more is rejected
/// (thrown) rather than silently truncated — generous enough for real traffic.
const MAX_HEADERS: usize = 128;

/// The plain-data result of parsing a response head — no VM/`Gc`, so it's unit-testable
/// without an arena. `head_len` is the byte length of the head (status line + headers +
/// the terminating CRLF CRLF); the body begins at that offset in the buffer.
#[derive(Debug, PartialEq)]
pub struct ParsedHead {
    pub code: u16,
    pub reason: String,
    pub head_len: usize,
    pub headers: Vec<(String, String)>,
}

/// Parse an HTTP/1.1 response head from `buf`. `Ok(None)` means the head is not yet
/// complete (read more bytes and retry); `Ok(Some(_))` is a complete head; `Err` is a
/// malformed head (or too many headers). Header values are decoded lossily (practically
/// always ASCII). A thin wrapper over `httparse`.
pub fn parse_head(buf: &[u8]) -> Result<Option<ParsedHead>, String> {
    let mut headers = [httparse::EMPTY_HEADER; MAX_HEADERS];
    let mut resp = httparse::Response::new(&mut headers);
    match resp.parse(buf) {
        Ok(httparse::Status::Complete(head_len)) => Ok(Some(ParsedHead {
            code: resp.code.unwrap_or(0),
            reason: resp.reason.unwrap_or("").to_string(),
            head_len,
            headers: resp
                .headers
                .iter()
                .map(|h| {
                    (
                        h.name.to_string(),
                        String::from_utf8_lossy(h.value).into_owned(),
                    )
                })
                .collect(),
        })),
        Ok(httparse::Status::Partial) => Ok(None),
        Err(e) => Err(format!("malformed response head: {e}")),
    }
}

/// The plain-data result of parsing a request head — the request-side mirror of
/// [`ParsedHead`], for the server. `version` is the HTTP/1.x minor (0 or 1); `target`
/// is the raw request-target as sent (no decoding or normalization here).
#[derive(Debug, PartialEq)]
pub struct ParsedRequestHead {
    pub method: String,
    pub target: String,
    pub version: u8,
    pub head_len: usize,
    pub headers: Vec<(String, String)>,
}

/// Parse an HTTP/1.1 request head from `buf`. Same contract as [`parse_head`]:
/// `Ok(None)` = incomplete, `Err` = malformed (or too many headers). A thin wrapper
/// over `httparse`.
pub fn parse_request_head(buf: &[u8]) -> Result<Option<ParsedRequestHead>, String> {
    let mut headers = [httparse::EMPTY_HEADER; MAX_HEADERS];
    let mut req = httparse::Request::new(&mut headers);
    match req.parse(buf) {
        Ok(httparse::Status::Complete(head_len)) => Ok(Some(ParsedRequestHead {
            method: req.method.unwrap_or("").to_string(),
            target: req.path.unwrap_or("").to_string(),
            version: req.version.unwrap_or(1),
            head_len,
            headers: req
                .headers
                .iter()
                .map(|h| {
                    (
                        h.name.to_string(),
                        String::from_utf8_lossy(h.value).into_owned(),
                    )
                })
                .collect(),
        })),
        Ok(httparse::Status::Partial) => Ok(None),
        Err(e) => Err(format!("malformed request head: {e}")),
    }
}

/// `[HTTP]Parser` — the one piece of the HTTP/1.1 client and server that isn't pure
/// Quoin: a thin native wrapper over `httparse`. Everything else (URL parsing, request
/// building, body framing, the `[HTTP]Client`/`Request`/`Response` classes and the
/// `[HTTP]Server` in `qnlib/net/http_server.qn`) lives in `qnlib/net/*` (loaded on
/// demand via `use std:net/...`), driving `TcpSocket`/`TlsSocket` directly.
pub fn build_http_parser_class() -> NativeClassBuilder {
    NativeClassBuilder::new("[HTTP]Parser", Some("Object"))
        .abstract_class()
        .class_doc(
            "Internal: the native HTTP/1.1 head parser under `[HTTP]Client` and \
             `[HTTP]Server` (`use std:net/http`) — a thin wrapper over `httparse`. Programs \
             normally use those classes, not this one.",
        )
        // parseHead: bytes -> nil if the response head is not complete yet, else
        // #( statusInt reasonStr headLenInt headers ), where `headers` is a list of
        // #( nameStr valueStr ) pairs. The body begins at `headLenInt` in `bytes`.
        .sdk_typed_class_method("parseHead:", &["Bytes"], |host, _receiver, args| {
            let bytes = arg!(args, Bytes, 0);
            let buf: &[u8] = &bytes;
            match parse_head(buf) {
                Ok(None) => Ok(host.new_nil()),
                Ok(Some(head)) => {
                    let header_vals: Vec<Value> = head
                        .headers
                        .into_iter()
                        .map(|(name, value)| {
                            let n = host.new_string(name);
                            let v = host.new_string(value);
                            host.new_list(vec![n, v])
                        })
                        .collect();
                    let headers_list = host.new_list(header_vals);
                    let status = host.new_int(head.code as i64);
                    let reason = host.new_string(head.reason);
                    let head_len = host.new_int(head.head_len as i64);
                    Ok(host.new_list(vec![status, reason, head_len, headers_list]))
                }
                Err(msg) => Err(QuoinError::ParseError(format!(
                    "[HTTP]Parser.parseHead:: {msg}"
                ))),
            }
        })
        .doc(
            "Parse an HTTP/1.1 *response* head from Bytes: nil while the head is incomplete \
             (read more and retry), else `#( status reason headLen headers )` where `headers` \
             is an order- and duplicate-preserving List of `#( name value )` pairs and the \
             body begins at byte `headLen`. Malformed input throws a ParseError.\n\n\
             ```\n\
             [HTTP]Parser.parseHead:'HTTP/1.1 200 OK\\r\\nContent-Length: 5\\r\\n\\r\\nhello'.asBytes\n\
             \"* -> #(200 OK 38 #(#(Content-Length 5)))\n\
             ```",
        )
        // parseRequestHead: bytes -> nil if the request head is not complete yet, else
        // #( methodStr targetStr versionInt headers ), where `versionInt` is the
        // HTTP/1.x minor (0|1), `targetStr` is the raw request-target, and `headers`
        // is the same order/duplicate-preserving list of #( nameStr valueStr ) pairs
        // as parseHead:.
        .sdk_typed_class_method("parseRequestHead:", &["Bytes"], |host, _receiver, args| {
            let bytes = arg!(args, Bytes, 0);
            let buf: &[u8] = &bytes;
            match parse_request_head(buf) {
                Ok(None) => Ok(host.new_nil()),
                Ok(Some(head)) => {
                    let header_vals: Vec<Value> = head
                        .headers
                        .into_iter()
                        .map(|(name, value)| {
                            let n = host.new_string(name);
                            let v = host.new_string(value);
                            host.new_list(vec![n, v])
                        })
                        .collect();
                    let headers_list = host.new_list(header_vals);
                    let method = host.new_string(head.method);
                    let target = host.new_string(head.target);
                    let version = host.new_int(head.version as i64);
                    Ok(host.new_list(vec![method, target, version, headers_list]))
                }
                Err(msg) => Err(QuoinError::ParseError(format!(
                    "[HTTP]Parser.parseRequestHead:: {msg}"
                ))),
            }
        })
        .doc(
            "Parse an HTTP/1.1 *request* head from Bytes (the server-side mirror of \
             `parseHead:`): nil while incomplete, else `#( method target version headers )` \
             where `version` is the HTTP/1.x minor (0 or 1) and `target` is the raw \
             request-target, undecoded.\n\n\
             ```\n\
             [HTTP]Parser.parseRequestHead:'GET /x HTTP/1.1\\r\\nHost: a\\r\\n\\r\\n'.asBytes\n\
             \"* -> #(GET /x 1 #(#(Host a)))\n\
             ```",
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complete_head_with_content_length() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nContent-Type: text/plain\r\n\r\nhello";
        let head = parse_head(raw).unwrap().expect("complete");
        assert_eq!(head.code, 200);
        assert_eq!(head.reason, "OK");
        assert_eq!(head.head_len, raw.len() - 5); // body "hello" is the remainder
        assert_eq!(
            head.headers,
            vec![
                ("Content-Length".to_string(), "5".to_string()),
                ("Content-Type".to_string(), "text/plain".to_string()),
            ]
        );
    }

    #[test]
    fn partial_head_returns_none() {
        // No terminating CRLF CRLF yet.
        assert_eq!(parse_head(b"HTTP/1.1 200 OK\r\nContent-Len").unwrap(), None);
    }

    #[test]
    fn malformed_head_errors() {
        assert!(parse_head(b"not http at all\r\n\r\n").is_err());
    }

    #[test]
    fn complete_request_head() {
        let raw =
            b"POST /users?active=1 HTTP/1.1\r\nHost: example.com\r\nContent-Length: 5\r\n\r\nhello";
        let head = parse_request_head(raw).unwrap().expect("complete");
        assert_eq!(head.method, "POST");
        assert_eq!(head.target, "/users?active=1");
        assert_eq!(head.version, 1);
        assert_eq!(head.head_len, raw.len() - 5); // body "hello" is the remainder
        assert_eq!(
            head.headers,
            vec![
                ("Host".to_string(), "example.com".to_string()),
                ("Content-Length".to_string(), "5".to_string()),
            ]
        );
    }

    #[test]
    fn request_head_version_1_0() {
        let head = parse_request_head(b"GET / HTTP/1.0\r\n\r\n")
            .unwrap()
            .expect("complete");
        assert_eq!(head.version, 0);
        assert_eq!(head.method, "GET");
        assert_eq!(head.target, "/");
        assert!(head.headers.is_empty());
    }

    #[test]
    fn request_head_preserves_duplicate_headers_in_order() {
        let raw = b"GET / HTTP/1.1\r\nAccept: text/html\r\nX-Tag: a\r\nX-Tag: b\r\n\r\n";
        let head = parse_request_head(raw).unwrap().expect("complete");
        assert_eq!(
            head.headers,
            vec![
                ("Accept".to_string(), "text/html".to_string()),
                ("X-Tag".to_string(), "a".to_string()),
                ("X-Tag".to_string(), "b".to_string()),
            ]
        );
    }

    #[test]
    fn partial_request_head_returns_none() {
        // No terminating CRLF CRLF yet.
        assert_eq!(
            parse_request_head(b"GET /index HTTP/1.1\r\nHos").unwrap(),
            None
        );
        // An empty buffer is just "read more".
        assert_eq!(parse_request_head(b"").unwrap(), None);
    }

    #[test]
    fn malformed_request_head_errors() {
        // HTTP/0.9-style line (no version) is rejected, as is plain garbage.
        assert!(parse_request_head(b"GET /\r\n\r\n").is_err());
        assert!(parse_request_head(b"complete garbage\x01\r\n\r\n").is_err());
    }
}
