// Context sidebar: a native tree (not the webview) for the symbol under the cursor or the map
// selection. Works with the map closed.
import * as vscode from "vscode";
import type { BlameResult, ContextResult, HistoryFileResult, NodeRef } from "../../../protocol/generated/protocol.ts";
import type { Session } from "../session.ts";
import { toRelative } from "../paths.ts";

type Item = Section | Ref | Fact;

class Section {
  readonly label: string;
  readonly refs: NodeRef[];
  readonly total: number;
  readonly notes: (string | undefined)[];
  readonly kind: "section" = "section";
  constructor(label: string, refs: NodeRef[], total = refs.length, notes: (string | undefined)[] = []) {
    this.label = label;
    this.refs = refs;
    this.total = total;
    this.notes = notes;
  }
}

class Ref {
  readonly kind: "ref" = "ref";
  readonly ref: NodeRef;
  /** Extra description (co-change partners: shared commits, hidden). */
  readonly note: string | undefined;
  constructor(ref: NodeRef, note?: string) {
    this.ref = ref;
    this.note = note;
  }
}

class Fact {
  readonly kind: "fact" = "fact";
  readonly label: string;
  readonly detail: string;
  constructor(label: string, detail: string) {
    this.label = label;
    this.detail = detail;
  }
}

/** "ann@example.com" → "ann", "123+ann@users.noreply.github.com" → "ann". */
const short = (author: string) => (author.indexOf("@") > 0 ? author.slice(0, author.indexOf("@")) : author).replace(/^\d+\+/, "");

const pct = (load: number) => (load >= 0.5 ? `top ${Math.max(1, Math.round((1 - load) * 100))}%` : `bottom ${Math.max(1, Math.round(load * 100))}%`);

export class ContextView implements vscode.TreeDataProvider<Item>, vscode.Disposable {
  private readonly changed = new vscode.EventEmitter<Item | undefined>();
  readonly onDidChangeTreeData = this.changed.event;
  private ctx: ContextResult | undefined;
  private hist: HistoryFileResult | undefined;
  /** Blame per file, until the next history change. */
  private readonly blames = new Map<string, BlameResult>();
  private blamePending = "";
  private timer: NodeJS.Timeout | undefined;
  private lastKey = "";
  private readonly disposables: vscode.Disposable[] = [];
  private readonly session: Session;
  private readonly view: vscode.TreeView<Item>;

  constructor(session: Session) {
    this.session = session;
    this.view = vscode.window.createTreeView("compass.context", { treeDataProvider: this, showCollapseAll: true });
    this.disposables.push(
      this.view,
      this.changed,
      vscode.window.onDidChangeTextEditorSelection(() => this.schedule()),
      vscode.window.onDidChangeActiveTextEditor(() => this.schedule()),
      this.view.onDidChangeVisibility(() => this.schedule(0)),
    );
  }

  /** Shows a specific node (map selection, commands). */
  async showSlot(slot: number): Promise<void> {
    try {
      const client = await this.session.get();
      const ctx = await client.request("node/context", { slot, limit: 50 });
      const path = ctx.node.kind === "dir" ? "" : ctx.node.path;
      this.hist = path ? await client.request("history/file", { path }).catch(() => undefined) : undefined;
      this.ctx = ctx;
      this.lastKey = `slot:${slot}`;
      this.render();
      if (path && this.hist?.commits) void this.loadBlame(path);
    } catch {
      // Node gone or indexer restarting.
    }
  }

  /** Re-reads the current node after the graph changed. */
  refresh(): void {
    if (this.ctx) void this.showSlot(this.ctx.node.slot);
  }

  /** New commits: blame results are stale. */
  historyChanged(): void {
    this.blames.clear();
    this.refresh();
  }

  /** Ownership by lines (blame) for the current file; computed once per file, off the cursor path. */
  private async loadBlame(path: string): Promise<void> {
    if (this.blames.has(path) || this.blamePending === path) return;
    this.blamePending = path;
    try {
      const client = await this.session.get();
      const b = await client.request("history/blame", { path });
      this.blames.set(path, b);
      if (this.ctx?.node.path === path) this.render();
    } catch {
      // Not in git, or the file is new.
    } finally {
      if (this.blamePending === path) this.blamePending = "";
    }
  }

  private schedule(delay = 250): void {
    if (!this.view.visible) return;
    clearTimeout(this.timer);
    this.timer = setTimeout(() => void this.follow(), delay);
  }

  /** Follows the cursor: the innermost symbol at the caret. */
  private async follow(): Promise<void> {
    const editor = vscode.window.activeTextEditor;
    const root = this.session.workspaceRoot;
    if (!editor || !root || editor.document.uri.scheme !== "file") return;
    const rel = toRelative(root, editor.document.uri.fsPath);
    if (!rel) return;
    try {
      const client = await this.session.get();
      const r = await client.request("node/locate", { path: rel, line: editor.selection.active.line + 1 });
      if (!r.node) return;
      const key = `slot:${r.node.slot}`;
      if (key === this.lastKey) return;
      await this.showSlot(r.node.slot);
    } catch {
      // Indexer not ready yet.
    }
  }

  private render(): void {
    const n = this.ctx?.node;
    this.view.description = n ? `${n.kind} ${n.kind === "file" || n.kind === "dir" ? n.name : n.qualified}` : undefined;
    this.changed.fire(undefined);
  }

  getChildren(item?: Item): Item[] {
    const c = this.ctx;
    if (!c) return [];
    if (!item) {
      const n = c.node;
      const facts: Item[] = [];
      facts.push(new Fact("Load-bearing", `${pct(n.load)} · used by ${n.inCount} · uses ${n.outCount}`));
      if (n.complexity) facts.push(new Fact("Complexity", `${n.complexity} (cyclomatic)`));
      if (n.inCycle) facts.push(new Fact("Import cycle", "this file is part of one (Compass: Show Cycles)"));
      if (n.deadCandidate) facts.push(new Fact("Dead-code candidate", "nothing depends on it; not an entry point or exported"));
      const h = this.hist;
      if (h && h.commits) {
        facts.push(new Fact("Churn", `${h.churn} commit${h.churn === 1 ? "" : "s"} in the last 90 days · ${h.commits.toLocaleString("en-US")} in all`));
        const blame = this.blames.get(n.path);
        const lines = blame?.authors.reduce((s, a) => s + a.lines, 0) ?? 0;
        if (blame && lines > 0) {
          const top = blame.authors.slice(0, 3).map((a) => `${short(a.author)} ${Math.round((a.lines / lines) * 100)}%`);
          facts.push(new Fact("Owner", `${top.join(" · ")} of lines`));
        } else if (h.owner) {
          facts.push(new Fact("Owner", `${short(h.owner)} · ${Math.round(h.ownerShare * 100)}% of commits`));
        }
      }
      const sections = [
        new Section("Callers", c.callers),
        new Section("Callees", c.callees),
        new Section("Importers", c.importers),
        new Section("Imports", c.imports),
        new Section("Tests that reach it", c.tests),
        new Section("Blast radius", c.blast, c.blastCount),
        new Section(
          "Changes with",
          (h?.partners ?? []).map((p) => p.node),
          h?.partners.length ?? 0,
          (h?.partners ?? []).map((p) => `${Math.round(p.degree * 100)}% · ${p.shared} commits${p.hidden ? " · no code link" : ""}`),
        ),
      ].filter((s) => s.total > 0);
      return [...facts, ...sections];
    }
    if (item.kind === "section") return item.refs.map((r, i) => new Ref(r, item.notes[i]));
    return [];
  }

  getTreeItem(item: Item): vscode.TreeItem {
    if (item.kind === "fact") {
      const t = new vscode.TreeItem(item.label, vscode.TreeItemCollapsibleState.None);
      t.description = item.detail;
      t.accessibilityInformation = { label: `${item.label}: ${item.detail}` };
      return t;
    }
    if (item.kind === "section") {
      const more = item.total > item.refs.length ? ` (showing ${item.refs.length})` : "";
      const t = new vscode.TreeItem(`${item.label}`, item.label === "Callers" || item.label === "Callees" ? vscode.TreeItemCollapsibleState.Expanded : vscode.TreeItemCollapsibleState.Collapsed);
      t.description = `${item.total.toLocaleString("en-US")}${more}`;
      t.accessibilityInformation = { label: `${item.label}, ${item.total} items` };
      return t;
    }
    const r = item.ref;
    const t = new vscode.TreeItem(r.name, vscode.TreeItemCollapsibleState.None);
    t.description = item.note ?? (r.kind === "file" ? r.path : `${r.path}:${r.line}`);
    t.tooltip = `${r.kind} ${r.name}\n${r.path}:${r.line}${item.note ? `\nchanges with it: ${item.note}` : ""}`;
    t.contextValue = "compassNode";
    t.accessibilityInformation = { label: `${r.kind} ${r.name} in ${r.path} line ${r.line}` };
    t.command = { command: "compass.openNodeRef", title: "Open", arguments: [r] };
    return t;
  }

  dispose(): void {
    clearTimeout(this.timer);
    for (const d of this.disposables.splice(0)) d.dispose();
  }
}

/** Opens a NodeRef in the editor at its line. */
export async function openNodeRef(root: string, r: NodeRef): Promise<void> {
  if (r.kind === "dir") return;
  const uri = vscode.Uri.joinPath(vscode.Uri.file(root), r.path);
  const pos = new vscode.Position(Math.max(0, r.line - 1), 0);
  await vscode.window.showTextDocument(uri, { selection: new vscode.Range(pos, pos), preview: true });
}
