// Forwards file-system changes (saves, creates, deletes, branch switches) to the indexer.
// Events are coalesced for 30 ms so a branch switch arrives as one batch. Commits, checkouts
// and pulls (writes to .git/logs/HEAD) refresh git history, coalesced for 1 s.
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
  private historyTimer: NodeJS.Timeout | undefined;
  /** Fired after git history was refreshed with new commits. */
  readonly onHistory = new vscode.EventEmitter<void>();

  constructor(session: Session) {
    this.session = session;
    const w = vscode.workspace.createFileSystemWatcher("**/*");
    const add = (uri: vscode.Uri) => this.add(uri);
    this.disposables.push(w, w.onDidChange(add), w.onDidCreate(add), w.onDidDelete(add), vscode.workspace.onDidSaveTextDocument((d) => add(d.uri)));
    const root = session.workspaceRoot;
    if (root) {
      const head = vscode.workspace.createFileSystemWatcher(new vscode.RelativePattern(vscode.Uri.file(root), ".git/logs/HEAD"));
      const commit = () => this.scheduleHistory();
      this.disposables.push(head, head.onDidChange(commit), head.onDidCreate(commit), this.onHistory);
    }
  }

  private scheduleHistory(): void {
    if (!this.session.serverInfo) return;
    clearTimeout(this.historyTimer);
    this.historyTimer = setTimeout(() => void this.refreshHistory(), 1000);
  }

  private async refreshHistory(): Promise<void> {
    try {
      const client = await this.session.get();
      const r = await client.request("history/refresh", null);
      if (r.changed) this.onHistory.fire();
    } catch (e) {
      this.session.log.warn(`history refresh failed: ${e instanceof Error ? e.message : e}`);
    }
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
    clearTimeout(this.historyTimer);
    for (const d of this.disposables.splice(0)) d.dispose();
  }
}
