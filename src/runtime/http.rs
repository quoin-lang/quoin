use crate::arg;
use crate::error::QuoinError;
use crate::value::{NativeClassBuilder, Value};

/// Max response headers we'll parse. A response with more is rejected (thrown) rather
/// than silently truncated — generous enough for real traffic.
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

/// `[HTTP]Parser` — the one piece of the HTTP/1.1 client that isn't pure Quoin: a thin
/// native wrapper over `httparse`. Everything else (URL parsing, request building, body
/// framing, the `[HTTP]Client`/`Request`/`Response` classes) lives in
/// `qnlib/net/http.qn` (loaded on demand via `use std:net/http`), driving
/// `TcpSocket`/`TlsSocket` directly.
pub fn build_http_parser_class() -> NativeClassBuilder {
    NativeClassBuilder::new("[HTTP]Parser", Some("Object"))
        // parseHead: bytes -> nil if the response head is not complete yet, else
        // #( statusInt reasonStr headLenInt headers ), where `headers` is a list of
        // #( nameStr valueStr ) pairs. The body begins at `headLenInt` in `bytes`.
        .typed_class_method("parseHead:", &["Bytes"], |vm, mc, _receiver, args| {
            let bytes = arg!(args, Bytes, 0);
            let buf: &[u8] = &bytes;
            match parse_head(buf) {
                Ok(None) => Ok(vm.new_nil(mc)),
                Ok(Some(head)) => {
                    let header_vals: Vec<Value> = head
                        .headers
                        .into_iter()
                        .map(|(name, value)| {
                            let n = vm.new_string(mc, name);
                            let v = vm.new_string(mc, value);
                            vm.new_list(mc, vec![n, v])
                        })
                        .collect();
                    let headers_list = vm.new_list(mc, header_vals);
                    let status = vm.new_int(mc, head.code as i64);
                    let reason = vm.new_string(mc, head.reason);
                    let head_len = vm.new_int(mc, head.head_len as i64);
                    Ok(vm.new_list(mc, vec![status, reason, head_len, headers_list]))
                }
                Err(msg) => Err(QuoinError::ParseError(format!(
                    "[HTTP]Parser.parseHead:: {msg}"
                ))),
            }
        })
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
}
