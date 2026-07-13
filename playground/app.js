// Playground page logic: owns the editor, the output pane, and the worker lifecycle.
// The VM runs in worker.js; Stop (and a pre-run safety cap) is worker.terminate().
// A SECOND wasm instance lives on this (main) thread purely for highlighting and
// Format: both stay responsive while a program runs, and survive Stop's terminate().
// Highlighting is the VM's own resilient highlighter (predictively completes
// incomplete input), so the editor's grammar can never drift from the language's.
import initMain, { fmt, highlight, highlight_stylesheet } from "./pkg/quoin_wasm.js";
import { examples } from "./examples.js";

const $ = (id) => document.getElementById(id);
const editor = $("editor");
const hl = $("highlight");
const output = $("output");
const result = $("result");
const status = $("status");
const runBtn = $("run");
const stopBtn = $("stop");
const formatBtn = $("format");
const examplesSel = $("examples");

let worker = null;
let running = false;
let hlReady = false;
let hlQueued = false;

function syncScroll() {
  hl.scrollTop = editor.scrollTop;
  hl.scrollLeft = editor.scrollLeft;
}

function refreshHighlight() {
  if (!hlReady || hlQueued) return;
  hlQueued = true;
  requestAnimationFrame(() => {
    hlQueued = false;
    // The trailing nbsp keeps a final empty line in the underlay the same height
    // as the textarea's, so vertical scroll extents match.
    hl.innerHTML = highlight(editor.value) + "&nbsp;";
    syncScroll();
  });
}

function setStatus(text) {
  status.textContent = text;
}

function appendOutput(cls, text) {
  const span = document.createElement("span");
  if (cls) span.className = cls;
  span.textContent = text;
  output.appendChild(span);
  output.scrollTop = output.scrollHeight;
}

function setResult(kind, text) {
  result.className = kind === "error" ? "error" : "";
  result.innerHTML = "";
  if (text === null || text === undefined || text === "") return;
  const tag = document.createElement("span");
  tag.className = "tag";
  tag.textContent = kind === "error" ? "error: " : "=> ";
  result.appendChild(tag);
  result.appendChild(document.createTextNode(text));
}

function setRunning(next) {
  running = next;
  runBtn.disabled = next || worker === null;
  stopBtn.disabled = !next;
  formatBtn.disabled = next || !hlReady;
}

function bootWorker() {
  worker = new Worker("worker.js", { type: "module" });
  worker.onmessage = (e) => {
    const msg = e.data;
    if (msg.type === "ready") {
      setStatus(`VM ready — quoin ${msg.version}, wasm`);
      setRunning(false);
    } else if (msg.type === "output") {
      appendOutput(msg.stream === "err" ? "err" : "", msg.text);
    } else if (msg.type === "done") {
      const o = msg.outcome;
      if (o.error !== null) setResult("error", o.error);
      else if (o.result !== null) setResult("ok", o.result);
      else if (o.exitCode !== 0) setResult("ok", `exited with status ${o.exitCode}`);
      else setResult("ok", "");
      setRunning(false);
    }
  };
  worker.onerror = (e) => {
    appendOutput("err", `worker error: ${e.message}\n`);
    setRunning(false);
  };
}

function run() {
  if (running || worker === null) return;
  output.textContent = "";
  setResult("ok", "");
  setRunning(true);
  setStatus("running…");
  worker.postMessage({ type: "run", source: editor.value, maxBatches: undefined });
}

function stop() {
  if (!running || worker === null) return;
  worker.terminate();
  worker = null;
  appendOutput("meta", "\n— stopped —\n");
  setStatus("restarting the VM…");
  setRunning(false);
  runBtn.disabled = true;
  bootWorker(); // a fresh VM; 'ready' re-enables Run
}

runBtn.addEventListener("click", run);
stopBtn.addEventListener("click", stop);
formatBtn.addEventListener("click", () => {
  if (!hlReady || running) return;
  const r = JSON.parse(fmt(editor.value));
  if (r.ok !== undefined) {
    editor.value = r.ok;
    setResult("ok", "");
    refreshHighlight();
  } else {
    setResult("error", r.error);
  }
});

editor.addEventListener("input", refreshHighlight);
editor.addEventListener("scroll", syncScroll);
editor.addEventListener("keydown", (e) => {
  if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
    e.preventDefault();
    run();
  } else if (e.key === "Tab") {
    e.preventDefault();
    const { selectionStart: s, selectionEnd: t, value } = editor;
    editor.value = value.slice(0, s) + "    " + value.slice(t);
    editor.selectionStart = editor.selectionEnd = s + 4;
    refreshHighlight(); // programmatic value changes fire no input event
  }
});

for (const name of Object.keys(examples)) {
  const opt = document.createElement("option");
  opt.value = name;
  opt.textContent = name;
  examplesSel.appendChild(opt);
}
examplesSel.addEventListener("change", () => {
  editor.value = examples[examplesSel.value];
  output.textContent = "";
  setResult("ok", "");
  refreshHighlight();
});

editor.value = examples[Object.keys(examples)[0]];
bootWorker();

// Boot the main-thread VM for highlighting + Format. Until it's ready the textarea
// keeps its own (uncolored) text — hl-on flips it transparent over the underlay.
(async () => {
  await initMain();
  const style = document.createElement("style");
  style.textContent = highlight_stylesheet();
  document.head.appendChild(style);
  hlReady = true;
  editor.classList.add("hl-on");
  if (!running) formatBtn.disabled = false;
  refreshHighlight();
})();
