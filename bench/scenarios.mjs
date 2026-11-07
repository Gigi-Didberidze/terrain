// Benchmark scenarios. Each drives the indexer over JSON-RPC and returns { metric: number }.
import { spawn } from "node:child_process";

/** Minimal JSON-RPC client (LSP framing), independent of the extension so the bench has no VS Code dependency. */
export class Rpc {
  constructor(binary) {
    this.proc = spawn(binary, ["--stdio"], { stdio: ["pipe", "pipe", "inherit"], windowsHide: true });
    this.buf = Buffer.alloc(0);
    this.next = 1;
    this.pending = new Map();
    this.listeners = new Map();
    this.proc.stdout.on("data", (c) => this.onData(c));
    this.exited = new Promise((r) => this.proc.on("exit", r));
  }
  onData(chunk) {
    this.buf = Buffer.concat([this.buf, chunk]);
    for (;;) {
      const h = this.buf.indexOf("\r\n\r\n");
      if (h < 0) return;
      const len = Number(/content-length:\s*(\d+)/i.exec(this.buf.subarray(0, h).toString())[1]);
      if (this.buf.length < h + 4 + len) return;
      const msg = JSON.parse(this.buf.subarray(h + 4, h + 4 + len).toString());
      this.buf = this.buf.subarray(h + 4 + len);
      if (msg.method) for (const l of this.listeners.get(msg.method) ?? []) l(msg.params);
      else {
        const p = this.pending.get(msg.id);
        this.pending.delete(msg.id);
        if (msg.error) p?.reject(new Error(msg.error.message));
        else p?.resolve(msg.result);
      }
    }
  }
  request(method, params = null) {
    const id = this.next++;
    const body = Buffer.from(JSON.stringify({ jsonrpc: "2.0", id, method, params }));
    this.proc.stdin.write(Buffer.concat([Buffer.from(`Content-Length: ${body.length}\r\n\r\n`), body]));
    return new Promise((resolve, reject) => this.pending.set(id, { resolve, reject }));
  }
  on(method, fn) {
    if (!this.listeners.has(method)) this.listeners.set(method, []);
    this.listeners.get(method).push(fn);
  }
  async close() {
    try {
      await this.request("shutdown");
    } catch {}
    const body = Buffer.from(JSON.stringify({ jsonrpc: "2.0", method: "exit" }));
    this.proc.stdin.write(Buffer.concat([Buffer.from(`Content-Length: ${body.length}\r\n\r\n`), body]));
    this.proc.stdin.end();
    await this.exited;
  }
}

export const scenarios = [
  {
    name: "startup",
    async run({ binary, root, store }) {
      // Warm-up spawn: the first launch of a fresh binary is dominated by the OS scanning it.
      const warm = new Rpc(binary);
      await warm.request("initialize", { workspaceRoot: root, storePath: `${store}/warm.db`, clientVersion: "bench" });
      await warm.close();
      const t0 = performance.now();
      const rpc = new Rpc(binary);
      await rpc.request("initialize", { workspaceRoot: root, storePath: `${store}/terrain.db`, clientVersion: "bench" });
      const t1 = performance.now();
      await rpc.close();
      return { "startup.spawnToInitializedMs": t1 - t0 };
    },
  },
];

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const pct = (xs, p) => {
  const s = [...xs].sort((a, b) => a - b);
  return s[Math.min(s.length - 1, Math.floor((p / 100) * s.length))];
};

/** Waits until the indexer reports the idle phase. */
function untilIdle(rpc) {
  return new Promise((resolve) => {
    rpc.on("index/progress", (p) => {
      if (p.phase === "idle") resolve();
    });
  });
}

function firstChange(rpc) {
  return new Promise((resolve) => rpc.on("map/changed", () => resolve(performance.now())));
}

/** Tiles covering a 1280x800 viewport fitted to the whole map (zoom 2). */
const FIRST_VIEW = [];
for (let y = 0; y < 3; y++) for (let x = 0; x < 4; x++) FIRST_VIEW.push({ z: 2, x, y });

scenarios.push({
  name: "index",
  async run({ binary, root, store }) {
    const { execFileSync } = await import("node:child_process");
    const { readdirSync, statSync, readFileSync, writeFileSync } = await import("node:fs");
    const { join } = await import("node:path");
    // Warm-up read so "cold" measures indexing, not the OS fetching files from disk for the
    // first time (on Windows the first read after a clone is dominated by antivirus scanning).
    execFileSync(binary, ["index", root], { stdio: "ignore" });
    const metrics = {};
    const db = join(store, "terrain.db");

    // Cold index.
    {
      const t0 = performance.now();
      const rpc = new Rpc(binary);
      const idle = untilIdle(rpc);
      const first = firstChange(rpc);
      await rpc.request("initialize", { workspaceRoot: root, storePath: db, clientVersion: "bench" });
      metrics["cold.firstMapMs"] = (await first) - t0;
      await idle;
      metrics["cold.indexMs"] = performance.now() - t0;
      const st = await rpc.request("index/status");
      metrics["memory.rssMB"] = st.rssMb;
      metrics["files"] = st.files;

      // Incremental: append a line to a mid-sized source file, then restore it.
      const files = [];
      const walk = (d, depth) => {
        if (files.length > 400 || depth > 6) return;
        for (const e of readdirSync(d, { withFileTypes: true })) {
          if (e.name.startsWith(".") || e.name === "node_modules") continue;
          const p = join(d, e.name);
          if (e.isDirectory()) walk(p, depth + 1);
          else if (/\.(py|ts|c|go|rs|java|rb)$/.test(e.name)) files.push(p);
        }
      };
      walk(root, 0);
      const sizes = files.map((f) => [f, statSync(f).size]).sort((a, b) => a[1] - b[1]);
      const [target] = sizes[Math.floor(sizes.length / 2)] ?? [];
      if (target) {
        const rel = target.slice(root.length + 1).split("\\").join("/");
        const original = readFileSync(target);
        const samples = [];
        try {
          for (let i = 0; i < 5; i++) {
            writeFileSync(target, Buffer.concat([original, Buffer.from(`\n// terrain bench ${i}\n`)]));
            const t = performance.now();
            const changed = firstChange(rpc);
            const r = await rpc.request("files/changed", { paths: [rel] });
            await Promise.race([changed, sleep(1000)]);
            samples.push(Math.max(r.elapsedMs, performance.now() - t));
          }
        } finally {
          writeFileSync(target, original);
          await rpc.request("files/changed", { paths: [rel] });
        }
        metrics["incremental.saveOneFileMs"] = pct(samples, 50);
      }

      // Search: prefixes of real file names, one request per keystroke.
      const names = files.map((f) => f.split(/[\/]/).pop().replace(/\.\w+$/, "")).filter((n) => n.length >= 4);
      const lat = [];
      for (let i = 0; i < Math.min(40, names.length); i++) {
        const name = names[(i * 7919) % names.length];
        for (let k = 1; k <= Math.min(6, name.length); k++) {
          const t = performance.now();
          await rpc.request("search", { query: name.slice(0, k), limit: 30 });
          lat.push(performance.now() - t);
        }
      }
      if (lat.length) metrics["search.p99Ms"] = pct(lat, 99);

      // Neighbourhood: the most load-bearing files (the densest neighbourhoods) plus symbols
      // found by search, each opened as a reader would.
      const slots = new Set();
      const top = await rpc.request("analysis/summary", { limit: 20 });
      for (const r of top.topLoadBearing) slots.add(r.node.slot);
      for (let i = 0; i < Math.min(40, names.length); i++) {
        const hits = await rpc.request("search", { query: names[(i * 104729) % names.length], limit: 3 });
        for (const h of hits.hits) slots.add(h.slot);
      }
      const nb = [];
      for (const slot of slots) {
        const t = performance.now();
        await rpc.request("view/neighbourhood", { slot });
        nb.push(performance.now() - t);
      }
      if (nb.length) metrics["neighbourhood.p99Ms"] = pct(nb, 99);
      await rpc.close();
    }

    // Warm start: map visible = initialize, first map/changed, first view's tiles drawn.
    {
      const t0 = performance.now();
      const rpc = new Rpc(binary);
      const idle = untilIdle(rpc);
      const first = firstChange(rpc);
      await rpc.request("initialize", { workspaceRoot: root, storePath: db, clientVersion: "bench" });
      await first;
      await rpc.request("map/tiles", { tiles: FIRST_VIEW });
      metrics["warm.mapVisibleMs"] = performance.now() - t0;
      await idle;
      await rpc.close();
    }
    return metrics;
  },
});
