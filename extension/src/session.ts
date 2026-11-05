// Owns the indexer process for one workspace. The indexer is started lazily on first use,
// so extension activation stays cheap (budget: < 20 ms).
import * as vscode from "vscode";
import { existsSync } from "node:fs";
import { join } from "node:path";
import { IndexerClient } from "./indexerClient.ts";
import type { InitializeResult } from "../../protocol/generated/protocol.ts";

export class Session implements vscode.Disposable {
  private client: IndexerClient | undefined;
  private ready: Promise<IndexerClient> | undefined;
  private readonly startListeners: ((c: IndexerClient) => void)[] = [];
  serverInfo: InitializeResult | undefined;
  readonly log: vscode.LogOutputChannel;
  private readonly context: vscode.ExtensionContext;

  constructor(context: vscode.ExtensionContext) {
    this.context = context;
    this.log = vscode.window.createOutputChannel("Compass", { log: true });
  }

  get workspaceRoot(): string | undefined {
    return vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;
  }

  /** Called for every new indexer process (initial start and restarts). */
  onStart(listener: (c: IndexerClient) => void): void {
    this.startListeners.push(listener);
    if (this.client?.running) listener(this.client);
  }

  /** Returns a started, initialized client. Safe to call repeatedly. */
  get(): Promise<IndexerClient> {
    if (this.ready && this.client?.running) return this.ready;
    this.ready = this.start();
    this.ready.catch(() => (this.ready = undefined));
    return this.ready;
  }

  private async start(): Promise<IndexerClient> {
    const root = this.workspaceRoot;
    if (!root) throw new Error("Open a folder to map it.");
    const binary = this.resolveBinary();
    const storeDir = (this.context.storageUri ?? this.context.globalStorageUri).fsPath;
    await vscode.workspace.fs.createDirectory(vscode.Uri.file(storeDir));
    const client = new IndexerClient(binary);
    client.onLog = (line) => this.log.appendLine(line);
    client.onExit = (code) => this.log.appendLine(`indexer exited (${code})`);
    client.start();
    this.client = client;
    this.serverInfo = await client.request("initialize", {
      workspaceRoot: root,
      storePath: join(storeDir, "compass.db"),
      clientVersion: String(this.context.extension.packageJSON.version ?? "0.0.0"),
    });
    this.log.appendLine(`indexer ${this.serverInfo.serverVersion} ready (${binary})`);
    for (const l of this.startListeners) l(client);
    return client;
  }

  private resolveBinary(): string {
    const exe = process.platform === "win32" ? "compass-indexer.exe" : "compass-indexer";
    const configured = vscode.workspace.getConfiguration("compass").get<string>("indexerPath");
    const ext = this.context.extensionPath;
    const candidates = [
      configured,
      join(ext, "bin", exe),
      join(ext, "..", "indexer", "target", "release", exe),
      join(ext, "..", "indexer", "target", "debug", exe),
    ].filter((p): p is string => !!p);
    const found = candidates.find((p) => existsSync(p));
    if (!found) throw new Error(`compass-indexer binary not found. Looked in:\n${candidates.join("\n")}`);
    return found;
  }

  async restart(): Promise<IndexerClient> {
    await this.client?.stop();
    this.client = undefined;
    this.ready = undefined;
    return this.get();
  }

  dispose(): void {
    void this.client?.stop();
    this.log.dispose();
  }
}
