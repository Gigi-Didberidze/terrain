// Forwards file-system changes (saves, creates, deletes, branch switches) to the indexer.
// Events are coalesced for 30 ms so a branch switch arrives as one batch.
import * as vscode from "vscode";
import type { Session } from "./session.ts";
import { toRelative } from "./paths.ts";

export class Watcher implements vscode.Disposable {
  private readonly pending = new Set<string>();
  private timer: NodeJS.Timeout | undefined;
  private readonly disposables: vscode.Disposable[] = [];
  private readonly session: Session;
  /** Last measured indexer-side update time, for the status bar and benchmarks. */
  lastUpdateMs = 0;

  constructor(session: Session) {
    this.session = session;
    const w = vscode.workspace.createFileSystemWatcher("**/*");
    const add = (uri: vscode.Uri) => this.add(uri);
    this.disposables.push(w, w.onDidChange(add), w.onDidCreate(add), w.onDidDelete(add), vscode.workspace.onDidSaveTextDocument((d) => add(d.uri)));
  }

  private add(uri: vscode.Uri): void {
    const root = this.session.workspaceRoot;
    if (!root || uri.scheme !== "file") return;
    const rel = toRelative(root, uri.fsPath);
    if (!rel || rel.startsWith(".git/") || rel.includes("/node_modules/") || rel.startsWith("node_modules/")) return;
    // Before the indexer runs, the cold index will see every change anyway.
    if (!this.session.serverInfo) return;
    this.pending.add(rel);
    clearTimeout(this.timer);
    this.timer = setTimeout(() => void this.flush(), 30);
  }

  private async flush(): Promise<void> {
    if (!this.session.serverInfo || this.pending.size === 0) return;
    const paths = [...this.pending];
    this.pending.clear();
    try {
      const client = await this.session.get();
      const r = await client.request("files/changed", { paths });
      this.lastUpdateMs = r.elapsedMs;
    } catch (e) {
      this.session.log.warn(`file update failed: ${e instanceof Error ? e.message : e}`);
    }
  }

  dispose(): void {
    clearTimeout(this.timer);
    for (const d of this.disposables.splice(0)) d.dispose();
  }
}
