#!/usr/bin/env node
// End-to-end smoke test for the playground wasm bundle (build it first with
// scripts/build_wasm.sh). Node and the browser share the --target web bundle;
// this drives the real wasm binary: embedded stdlib boot, JS entropy, JS clock,
// output capture, and the JSON outcome shape.
import { readFileSync } from "node:fs";
import init, {
  run,
  fmt,
  highlight,
  highlight_stylesheet,
  version,
} from "../playground/pkg/quoin_wasm.js";

const wasmBytes = readFileSync(
  new URL("../playground/pkg/quoin_wasm_bg.wasm", import.meta.url),
);
await init({ module_or_path: wasmBytes });

let failures = 0;
function check(name, cond, detail) {
  if (cond) {
    console.log(`ok   ${name}`);
  } else {
    failures += 1;
    console.error(`FAIL ${name}: ${detail}`);
  }
}

function runQ(source, maxBatches) {
  const chunks = { out: "", err: "" };
  const outcome = JSON.parse(
    run(source, maxBatches, (stream, text) => {
      chunks[stream] += text;
    }),
  );
  return { outcome, ...chunks };
}

// Hello world: print streams through the capture seam, result renders.
{
  const { outcome, out } = runQ("'hello from wasm'.print; 6 * 7");
  check("prints stream to the callback", out.includes("hello from wasm"), out);
  check("final value renders", outcome.result === "42", JSON.stringify(outcome));
  check("no error", outcome.error === null, JSON.stringify(outcome));
}

// Stdlib breadth: collections, strings, JSON — all pure-Rust deps.
{
  const { outcome } = runQ("JSON.generate:(#( 1 2 3 ).collect:{ |x| x * x })");
  check("stdlib JSON round-trip", outcome.result === "'[1,4,9]'", JSON.stringify(outcome));
}

// Entropy: uuid v4 goes through getrandom's wasm_js backend.
{
  const { outcome } = runQ("UUID.generateV4.s.length");
  check("JS entropy backend works (UUID)", outcome.result === "36", JSON.stringify(outcome));
}

// Clock: Instant reads web-time (performance.now) on wasm.
{
  const { outcome } = runQ("Instant.now.elapsed.asNanoseconds >= 0");
  check("JS clock works (Instant)", outcome.result === "true", JSON.stringify(outcome));
}

// Async primitives raise catchably without a scheduler.
{
  const { outcome } = runQ(
    "var got = 'not caught'; { Async.sleep:1 }.catch:{ |e| got = 'caught' }; got",
  );
  check("async raises catchably", outcome.result === "'caught'", JSON.stringify(outcome));
}

// Compile diagnostics arrive on the err stream.
{
  const { outcome, err } = runQ("undeclared = 1");
  check("compile error reported", outcome.error === "compile error", JSON.stringify(outcome));
  check("diagnostic on err stream", err.includes("error"), err);
}

// The batch budget stops a runaway loop.
{
  const { outcome } = runQ("{ true }.whileDo:{ 1 }", 10);
  check(
    "batch budget stops runaway loops",
    (outcome.error ?? "").includes("instruction budget"),
    JSON.stringify(outcome),
  );
}

// The formatter rides along.
{
  const formatted = JSON.parse(fmt("1+2"));
  check("fmt works", typeof formatted.ok === "string", JSON.stringify(formatted));
}

// The highlighter: grammar-true spans, escaped text, resilient on incomplete input.
{
  const html = highlight("'hi'.print \"* note");
  check("highlight emits token spans", html.includes("qn-string") && html.includes("qn-comment"), html);
  const partial = highlight("Foo <- { bar -> { 1 +");
  check("highlight survives incomplete input", typeof partial === "string" && partial.length > 0, partial);
  const escaped = highlight("'<script>'");
  check("highlight escapes markup", !escaped.includes("<script>"), escaped);
  const css = highlight_stylesheet();
  check("stylesheet covers the classes", css.includes(".qn-string") && css.includes("prefers-color-scheme: dark"), css.slice(0, 120));
}

console.log(`quoin-wasm ${version()}: ${failures === 0 ? "all checks passed" : `${failures} FAILED`}`);
process.exit(failures === 0 ? 0 : 1);
