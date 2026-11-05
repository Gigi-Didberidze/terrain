// Stand-in for the VS Code webview API: forwards requests to the harness server and turns
// server-sent events into the same messages the extension host would post.
(() => {
  const post = (msg) => window.postMessage(msg, "*");
  let state = {};
  try {
    state = JSON.parse(sessionStorage.getItem("compass-state") || "{}");
  } catch {}
  const rpc = async (method, params) => {
    const r = await fetch("/rpc", { method: "POST", body: JSON.stringify({ method, params }) });
    return r.json();
  };
  // Native base64 decoding where available; VS Code itself passes raw bytes, so this keeps the
  // harness from adding main-thread work the real webview never does.
  const b64 =
    typeof Uint8Array.fromBase64 === "function"
      ? (s) => Uint8Array.fromBase64(s)
      : (s) => {
          const bin = atob(s);
          const out = new Uint8Array(bin.length);
          for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
          return out;
        };
  window.__compass = { rpc };
  window.acquireVsCodeApi = () => ({
    async postMessage(m) {
      if (m.type === "ready") {
        const ev = new EventSource("/events");
        ev.onmessage = (e) => post(JSON.parse(e.data));
        await rpc("ping", null);
        post({ type: "status", state: "ready", text: "" });
        return;
      }
      if (m.type === "request") {
        const r = await rpc(m.method, m.params);
        if (r.error) return post({ type: "response", id: m.id, error: r.error });
        let result = r.result;
        if (m.method === "map/tiles") result = { epoch: result.epoch, tiles: result.tiles.map((t) => ({ ...t, data: b64(t.data) })) };
        post({ type: "response", id: m.id, result });
        return;
      }
      if (m.type === "log") console.log(`[compass] ${m.text}`);
      window.__compassLast = m;
    },
    getState: () => state,
    setState: (s) => {
      state = s;
      try {
        sessionStorage.setItem("compass-state", JSON.stringify(s));
      } catch {}
    },
  });
})();
