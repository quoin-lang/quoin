// The execution side of the playground: a module worker owning the wasm VM, so an
// infinite loop can never freeze the page — Stop is worker.terminate() from app.js.
// The same pkg/ bundle the smoke test drives under Node. Highlighting and Format
// run on the main thread's own instance (app.js), never through here.
import init, { run, version } from "./pkg/quoin_wasm.js";

await init();
postMessage({ type: "ready", version: version() });

onmessage = (e) => {
  const msg = e.data;
  if (msg.type === "run") {
    const outcome = JSON.parse(
      run(msg.source, msg.maxBatches, (stream, text) =>
        postMessage({ type: "output", stream, text }),
      ),
    );
    postMessage({ type: "done", outcome });
  }
};
