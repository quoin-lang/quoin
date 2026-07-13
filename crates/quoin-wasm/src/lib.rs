//! Browser bindings for the Quoin VM: the quoinlang.dev playground engine.
//!
//! A thin wasm-bindgen shell over [`quoin::direct_runner`] — the coroutine-less
//! driver that steps the interpreter on the caller's stack. The heavy lifting
//! (what is and isn't supported in a browser build) lives in the VM crate's
//! `cfg(target_arch = "wasm32")` gates; this crate just crosses the JS boundary:
//! strings in, a JSON outcome out, output streamed through a JS callback.
//!
//! Build with `scripts/build_wasm.sh` (wasm-pack, `--target web`); the same bundle
//! runs in the browser and under Node (`scripts/wasm_smoke.mjs`).

use quoin::direct_runner::{DirectConfig, run_source};
use quoin::vm::StdStream;
use wasm_bindgen::prelude::*;

#[wasm_bindgen(start)]
fn init() {
    console_error_panic_hook::set_once();
}

/// Parse and run `source` against the embedded stdlib.
///
/// `on_output` is called as `on_output(stream, text)` with `stream` `"out"` or
/// `"err"` for every captured chunk of program output (prints, compile
/// diagnostics), in order, while the program runs. `max_batches` caps execution
/// (in dispatch batches of `QN_BATCH` instructions each; pass `undefined` for no
/// cap) — the belt-and-suspenders alongside terminating the hosting worker.
///
/// Returns a JSON string: `{"result": string|null, "error": string|null,
/// "exitCode": number}` — `result` is the final expression pretty-rendered,
/// exactly one of `result`/`error` is non-null unless the program called
/// `Runtime.exit:`.
#[wasm_bindgen]
pub fn run(source: &str, max_batches: Option<u32>, on_output: &js_sys::Function) -> String {
    let cfg = DirectConfig {
        max_batches: max_batches.map(u64::from),
        ..DirectConfig::default()
    };
    let mut sink = |stream: StdStream, bytes: &[u8]| {
        let tag = match stream {
            StdStream::Out => "out",
            StdStream::Err => "err",
        };
        let text = String::from_utf8_lossy(bytes);
        // A throwing callback must not unwind into the VM; swallow it (the page
        // owns its own console noise).
        let _ = on_output.call2(
            &JsValue::NULL,
            &JsValue::from_str(tag),
            &JsValue::from_str(&text),
        );
    };
    let outcome = run_source("playground.qn", source, cfg, &mut sink);
    serde_json::json!({
        "result": outcome.result,
        "error": outcome.error,
        "exitCode": outcome.exit_code,
    })
    .to_string()
}

/// Format `source` with the canonical `qn fmt` style. Returns JSON:
/// `{"ok": string}` or `{"error": string}` (a parse error, verbatim).
#[wasm_bindgen]
pub fn fmt(source: &str) -> String {
    match quoin_fmt::format(source) {
        Ok(formatted) => serde_json::json!({ "ok": formatted }).to_string(),
        Err(e) => serde_json::json!({ "error": e.to_string() }).to_string(),
    }
}

/// The VM version baked into this bundle (the workspace version).
#[wasm_bindgen]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}
