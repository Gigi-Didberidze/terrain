// Spawns the terrain-indexer binary and speaks JSON-RPC 2.0 to it over stdio (LSP framing).
import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import type { Notifications, NotificationMethod, Requests, RequestMethod } from "../../protocol/generated/protocol.ts";
import { FrameDecoder, encodeFrame } from "./framing.ts";

type Pending = { resolve: (v: unknown) => void; reject: (e: Error) => void; method: string };
type Handler<M extends NotificationMethod> = (params: Notifications[M]) => void;

export class IndexerError extends Error {
  readonly code: number;
  constructor(code: number, message: string) {
    super(message);
    this.code = code;
  }
}

export class IndexerClient {
  private proc: ChildProcessWithoutNullStreams | undefined;
  private nextId = 1;
  private readonly pending = new Map<number, Pending>();
  private readonly handlers = new Map<string, Set<(p: unknown) => void>>();
  private readonly decoder = new FrameDecoder();
  private exited = false;
  onExit: ((code: number | null) => void) | undefined;
  onLog: ((line: string) => void) | undefined;

  readonly binaryPath: string;
  constructor(binaryPath: string) {
    this.binaryPath = binaryPath;
  }

  start(): void {
    const proc = spawn(this.binaryPath, ["--stdio"], { stdio: "pipe", windowsHide: true });
    this.proc = proc;
    proc.stdout.on("data", (chunk: Buffer) => {
      for (const body of this.decoder.push(chunk)) this.onMessage(body);
    });
    proc.stderr.on("data", (chunk: Buffer) => this.onLog?.(chunk.toString("utf8").trimEnd()));
    proc.on("exit", (code) => {
      this.exited = true;
      this.failAll(new Error(`indexer exited with code ${code}`));
      this.onExit?.(code);
    });
    proc.on("error", (err) => {
      this.exited = true;
      this.failAll(err);
      this.onLog?.(`failed to start indexer: ${err.message}`);
    });
  }

  get running(): boolean {
    return this.proc !== undefined && !this.exited;
  }

  request<M extends RequestMethod>(method: M, params: Requests[M]["params"]): Promise<Requests[M]["result"]> {
    if (!this.proc || this.exited) return Promise.reject(new Error("indexer is not running"));
    const id = this.nextId++;
    const promise = new Promise<Requests[M]["result"]>((resolve, reject) => {
      this.pending.set(id, { resolve: resolve as (v: unknown) => void, reject, method });
    });
    this.send({ jsonrpc: "2.0", id, method, params });
    return promise;
  }

  on<M extends NotificationMethod>(method: M, handler: Handler<M>): () => void {
    let set = this.handlers.get(method);
    if (!set) this.handlers.set(method, (set = new Set()));
    const h = handler as (p: unknown) => void;
    set.add(h);
    return () => set.delete(h);
  }

  async stop(): Promise<void> {
    if (!this.running) return;
    try {
      await Promise.race([this.request("shutdown", null), new Promise((r) => setTimeout(r, 500))]);
    } catch {
      // Already gone.
    }
    this.send({ jsonrpc: "2.0", method: "exit", params: null });
    this.proc?.stdin.end();
  }

  private failAll(err: Error): void {
    for (const p of this.pending.values()) p.reject(new Error(`${err.message} (during ${p.method})`));
    this.pending.clear();
  }

  private send(msg: unknown): void {
    this.proc?.stdin.write(encodeFrame(Buffer.from(JSON.stringify(msg), "utf8")));
  }

  private onMessage(body: Buffer): void {
    let msg: { id?: number; method?: string; params?: unknown; result?: unknown; error?: { code: number; message: string } };
    try {
      msg = JSON.parse(body.toString("utf8"));
    } catch {
      this.onLog?.("indexer sent malformed JSON");
      return;
    }
    if (msg.method !== undefined) {
      for (const h of this.handlers.get(msg.method) ?? []) h(msg.params);
      return;
    }
    if (typeof msg.id !== "number") return;
    const p = this.pending.get(msg.id);
    if (!p) return;
    this.pending.delete(msg.id);
    if (msg.error) p.reject(new IndexerError(msg.error.code, msg.error.message));
    else p.resolve(msg.result ?? null);
  }
}
