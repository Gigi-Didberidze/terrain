// Webview entry point. Plain TypeScript, no framework: the UI is small and must render instantly.
import "./main.css";
import type { HostToView, ViewToHost } from "../../protocol/webview.ts";

declare function acquireVsCodeApi(): { postMessage(m: ViewToHost): void };
const vscode = acquireVsCodeApi();

const app = document.getElementById("app")!;
const state = document.createElement("div");
state.className = "state";
state.setAttribute("role", "status");
app.append(state);

window.addEventListener("message", (ev: MessageEvent<HostToView>) => {
  const m = ev.data;
  switch (m.type) {
    case "status":
      state.dataset.kind = m.state;
      state.textContent = m.text;
      break;
    case "progress":
      state.dataset.kind = "progress";
      state.textContent = `${m.phase} ${m.done.toLocaleString("en-US")} / ${m.total.toLocaleString("en-US")}`;
      break;
  }
});

vscode.postMessage({ type: "ready" });
