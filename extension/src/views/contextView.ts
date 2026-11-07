// Context sidebar: a native tree (not the webview) for the symbol under the cursor or the map
// selection. Works with the map closed.
import * as vscode from "vscode";
import type { ContextResult, NodeRef } from "../../../protocol/generated/protocol.ts";
import type { Session } from "../session.ts";
import { toRelative } from "../paths.ts";

type Item = Section | Ref | Fact;

class Section {
  readonly label: string;
  readonly refs: NodeRef[];
  readonly total: number;
  readonly kind: "section" = "section";
  constructor(label: string, refs: NodeRef[], total = refs.length) {
    this.label = label;
    this.refs = refs;
    this.total = total;
  }
}

class Ref {
  readonly kind: "ref" = "ref";
  readonly ref: NodeRef;
  constructor(ref: NodeRef) {
    this.ref = ref;
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

const pct = (load: number) => (load >= 0.5 ? `top ${Math.max(1, Math.round((1 - load) * 100))}%` : `bottom ${Math.max(1, Math.round(load * 100))}%`);

export class ContextView implements vscode.TreeDataProvider<Item>, vscode.Disposable {
  private readonly changed = new vscode.EventEmitter<Item | undefined>();
  readonly onDidChangeTreeData = this.changed.event;
  private ctx: ContextResult | undefined;
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
      this.ctx = await client.request("node/context", { slot, limit: 50 });
      this.lastKey = `slot:${slot}`;
      this.render();
    } catch {
      // Node gone or indexer restarting.
    }
  }

  /** Re-reads the current node after the graph changed. */
  refresh(): void {
    if (this.ctx) void this.showSlot(this.ctx.node.slot);
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
      const sections = [
        new Section("Callers", c.callers),
        new Section("Callees", c.callees),
        new Section("Importers", c.importers),
        new Section("Imports", c.imports),
        new Section("Tests that reach it", c.tests),
        new Section("Blast radius", c.blast, c.blastCount),
      ].filter((s) => s.total > 0);
      return [...facts, ...sections];
    }
    if (item.kind === "section") return item.refs.map((r) => new Ref(r));
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
    t.description = r.kind === "file" ? r.path : `${r.path}:${r.line}`;
    t.tooltip = `${r.kind} ${r.name}\n${r.path}:${r.line}`;
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
