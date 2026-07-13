// The execution side of the playground: a module worker owning the wasm VM, so an
// infinite loop can never freeze the page — Stop is worker.terminate() from app.js.
// The same pkg/ bundle the smoke test drives under Node.
import init, { run, fmt, version } from "./pkg/quoin_wasm.js";

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
  } else if (msg.type === "fmt") {
    postMessage({ type: "fmt", result: JSON.parse(fmt(msg.source)) });
  }
};
