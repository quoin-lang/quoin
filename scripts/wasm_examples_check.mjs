#!/usr/bin/env node
// Runs every playground example against the real wasm bundle (build it first with
// scripts/build_wasm.sh) so a stale selector can't ship on the website.
import { readFileSync } from "node:fs";
import init, { run } from "../playground/pkg/quoin_wasm.js";
import { examples } from "../playground/examples.js";

const wasmBytes = readFileSync(
  new URL("../playground/pkg/quoin_wasm_bg.wasm", import.meta.url),
);
await init({ module_or_path: wasmBytes });

let failures = 0;
for (const [name, source] of Object.entries(examples)) {
  let err = "";
  const outcome = JSON.parse(
    run(source, undefined, (stream, text) => {
      if (stream === "err") err += text;
    }),
  );
  if (outcome.error !== null || err !== "") {
    failures += 1;
    console.error(`FAIL ${name}: ${outcome.error ?? ""}\n${err}`);
  } else {
    console.log(`ok   ${name} => ${outcome.result}`);
  }
}
process.exit(failures === 0 ? 0 : 1);
