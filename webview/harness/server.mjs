// Development and test harness: serves the real webview bundle outside VS Code, backed by a real
// indexer. Used for Playwright visual tests and for profiling the renderer in Chrome DevTools.
//
// Usage: node webview/harness/server.mjs <workspaceRoot> [--port 4710] [--binary path] [--store db]
// Then open http://localhost:4710/?theme=dark   (dark | light | hc)
import { createServer } from "node:http";
import { readFileSync, existsSync, mkdtempSync } from "node:fs";
import { dirname, join, extname } from "node:path";
import { fileURLToPath } from "node:url";
import { tmpdir } from "node:os";
import { Rpc } from "../../bench/scenarios.mjs";
import { THEMES } from "./themes.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const args = process.argv.slice(2);
const opt = (n, d) => {
  const i = args.indexOf(`--${n}`);
  return i >= 0 ? args[i + 1] : d;
};
const root = args.find((a, i) => !a.startsWith("--") && !args[i - 1]?.startsWith("--"));
if (!root) {
  console.error("usage: node webview/harness/server.mjs <workspaceRoot> [--port N] [--binary path]");
  process.exit(2);
}
const port = Number(opt("port", "4710"));
const exe = process.platform === "win32" ? "terrain-indexer.exe" : "terrain-indexer";
const binary = opt("binary", join(here, "..", "..", "indexer", "target", "release", exe));
const dist = join(here, "..", "..", "extension", "dist", "webview");
if (!existsSync(join(dist, "main.js"))) {
  console.error("build the webview first: npm run build -w webview");
  process.exit(2);
}

const rpc = new Rpc(binary);
// --store reuses an existing index (warm start); default is a fresh temporary store.
const store = opt("store", join(mkdtempSync(join(tmpdir(), "compass-harness-")), "terrain.db"));
const ready = rpc.request("initialize", { workspaceRoot: root, storePath: store, clientVersion: "harness" });
const clients = new Set();
const send = (msg) => {
  const data = `data: ${JSON.stringify(msg)}\n\n`;
  for (const c of clients) c.write(data);
};
rpc.on("index/progress", (p) => send({ type: "progress", phase: p.phase, done: p.done, total: p.total }));
rpc.on("map/changed", (p) => send({ type: "changed", epoch: p.epoch, layout: p.layout }));

const types = { ".js": "text/javascript", ".css": "text/css", ".map": "application/json" };

createServer(async (req, res) => {
  const url = new URL(req.url, `http://localhost:${port}`);
  if (url.pathname === "/") {
    const theme = THEMES[url.searchParams.get("theme") ?? "dark"] ?? THEMES.dark;
    const vars = Object.entries(theme.vars).map(([k, v]) => `--vscode-${k}: ${v};`).join("\n");
    res.writeHead(200, { "content-type": "text/html" });
    res.end(`<!doctype html><html><head><meta charset="utf-8">
<style>:root{${vars}}</style>
<link rel="stylesheet" href="/main.css"></head>
<body class="${theme.bodyClass}"><div id="app"></div>
<script src="/shim.js"></script><script type="module" src="/main.js"></script></body></html>`);
    return;
  }
  if (url.pathname === "/shim.js") {
    res.writeHead(200, { "content-type": "text/javascript" });
    res.end(readFileSync(join(here, "shim.js")));
    return;
  }
  if (url.pathname === "/events") {
    res.writeHead(200, { "content-type": "text/event-stream", "cache-control": "no-cache", connection: "keep-alive" });
    clients.add(res);
    req.on("close", () => clients.delete(res));
    return;
  }
  if (url.pathname === "/rpc" && req.method === "POST") {
    let body = "";
    for await (const chunk of req) body += chunk;
    const { method, params } = JSON.parse(body);
    try {
      await ready;
      const result = await rpc.request(method, params);
      res.writeHead(200, { "content-type": "application/json" });
      res.end(JSON.stringify({ result }));
    } catch (e) {
      res.writeHead(200, { "content-type": "application/json" });
      res.end(JSON.stringify({ error: String(e?.message ?? e) }));
    }
    return;
  }
  const file = join(dist, url.pathname.replace(/^\/+/, ""));
  if (file.startsWith(dist) && existsSync(file)) {
    res.writeHead(200, { "content-type": types[extname(file)] ?? "application/octet-stream" });
    res.end(readFileSync(file));
    return;
  }
  res.writeHead(404);
  res.end();
}).listen(port, () => console.log(`harness on http://localhost:${port}/?theme=dark  (root: ${root})`));
