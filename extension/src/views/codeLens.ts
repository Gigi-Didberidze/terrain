// CodeLens above functions, methods and classes: "12 callers · blast 48 · 3 tests".
// Off for files over 5,000 lines (and configurable).
import * as vscode from "vscode";
import type { Session } from "../session.ts";
import { toRelative } from "../paths.ts";

const MAX_LINES = 5000;

export class CompassCodeLens implements vscode.CodeLensProvider, vscode.Disposable {
  private readonly changed = new vscode.EventEmitter<void>();
  readonly onDidChangeCodeLenses = this.changed.event;
  private readonly session: Session;
  private timer: NodeJS.Timeout | undefined;

  constructor(session: Session) {
    this.session = session;
  }

  /** The graph changed: lenses refresh, debounced so a save doesn't flicker them. */
  refresh(): void {
    clearTimeout(this.timer);
    this.timer = setTimeout(() => this.changed.fire(), 800);
  }

  async provideCodeLenses(doc: vscode.TextDocument, token: vscode.CancellationToken): Promise<vscode.CodeLens[]> {
    if (!vscode.workspace.getConfiguration("compass").get<boolean>("codeLens.enabled", true)) return [];
    if (doc.lineCount > MAX_LINES || doc.uri.scheme !== "file") return [];
    const root = this.session.workspaceRoot;
    const rel = root ? toRelative(root, doc.uri.fsPath) : undefined;
    if (!rel) return [];
    try {
      const client = await this.session.get();
      const r = await client.request("file/lenses", { path: rel });
      if (token.isCancellationRequested) return [];
      return r.lenses.map((l) => {
        const parts = [`${l.callers} caller${l.callers === 1 ? "" : "s"}`, `blast ${l.blast}`, `${l.tests} test${l.tests === 1 ? "" : "s"}`];
        if (l.complexity >= 10) parts.push(`complexity ${l.complexity}`);
        const line = Math.max(0, Math.min(doc.lineCount - 1, l.line - 1));
        return new vscode.CodeLens(new vscode.Range(line, 0, line, 0), {
          title: parts.join(" · "),
          tooltip: "Compass: show what uses this and what it uses (Neighbourhood)",
          command: "compass.showNeighbourhood",
          arguments: [l.slot],
        });
      });
    } catch {
      return [];
    }
  }

  dispose(): void {
    clearTimeout(this.timer);
    this.changed.dispose();
  }
}
