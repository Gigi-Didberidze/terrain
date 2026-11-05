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
      await rpc.request("initialize", { workspaceRoot: root, storePath: `${store}/compass.db`, clientVersion: "bench" });
      const t1 = performance.now();
      await rpc.close();
      return { "startup.spawnToInitializedMs": t1 - t0 };
    },
  },
];
