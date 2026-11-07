// Entry point. Activation only registers commands, views and listeners; the indexer process
// starts lazily the first time something needs it (budget: activation < 20 ms).
import * as vscode from "vscode";
import type { CycleView, HiddenPair, MisplacedView, NodeRef } from "../../protocol/generated/protocol.ts";
import { Session } from "./session.ts";
import { MapController } from "./mapController.ts";
import { Watcher } from "./watcher.ts";
import { MapPanel } from "./panel.ts";
import { ContextView, openNodeRef } from "./views/contextView.ts";
import { TerrainCodeLens } from "./views/codeLens.ts";
import { pickReviewFile, ReviewStatus } from "./views/review.ts";
import { toRelative } from "./paths.ts";
import { LspBridge } from "./lspBridge.ts";

let session: Session | undefined;

export interface TerrainApi {
  session: Session;
  map: MapController;
  watcher: Watcher;
  context: ContextView;
  lsp: LspBridge;
  review: ReviewStatus;
  activationMs: number;
}

const CODE_LANGUAGES = ["typescript", "typescriptreact", "javascript", "javascriptreact", "python", "rust", "go", "java", "c", "cpp", "csharp", "kotlin", "ruby", "php", "swift", "lua"];

export function activate(context: vscode.ExtensionContext): TerrainApi {
  const t0 = performance.now();
  const s = new Session(context);
  session = s;
  const map = new MapController(context, s);
  const watcher = new Watcher(s);
  const contextView = new ContextView(s);
  const lens = new TerrainCodeLens(s);
  const review = new ReviewStatus(s);
  const lsp = new LspBridge(s);
  // Whatever the developer looks at gets its calls confirmed by the language server.
  contextView.onShow((slot) => lsp.enrich(slot));
  watcher.onHistory.event(() => review.refresh(0));
  watcher.onHistory.event(() => contextView.historyChanged());
  map.onSelection.event((slot) => {
    if (slot !== null) void contextView.showSlot(slot);
  });
  s.onStart((client) => {
    client.on("map/changed", () => {
      contextView.refresh();
      lens.refresh();
      review.refresh();
    });
    review.refresh(0);
  });
  const root = () => s.workspaceRoot ?? "";
  context.subscriptions.push(
    s,
    map,
    watcher,
    contextView,
    lens,
    review,
    lsp,
    vscode.languages.registerCodeLensProvider(
      CODE_LANGUAGES.map((language) => ({ language, scheme: "file" })),
      lens,
    ),
    vscode.commands.registerCommand("terrain.openMap", () => map.open()),
    vscode.commands.registerCommand("terrain.restartIndexer", () => map.restartIndexer()),
    vscode.commands.registerCommand("terrain.showLog", () => s.log.show()),
    vscode.commands.registerCommand("terrain.reindex", async () => {
      await s.restart();
      MapPanel.current?.dispose();
      await map.open();
    }),
    vscode.commands.registerCommand("terrain.searchMap", async () => {
      const panel = MapPanel.current ?? (await map.open());
      panel.post({ type: "command", command: "search" });
    }),
    vscode.commands.registerCommand("terrain.openNodeRef", (r: NodeRef) => openNodeRef(root(), r)),
    vscode.commands.registerCommand("terrain.revealNode", async (slot: number) => {
      await contextView.showSlot(slot);
      await vscode.commands.executeCommand("terrain.context.focus");
      if (MapPanel.current) await map.reveal(slot);
    }),
    vscode.commands.registerCommand("terrain.showNeighbourhood", async (slot?: number) => {
      const target = typeof slot === "number" ? slot : await slotAtCursor(s);
      if (target === undefined) {
        void vscode.window.showInformationMessage("Terrain: put the cursor in a function, class or file that is on the map.");
        return;
      }
      await map.command({ type: "command", command: "neighbourhood", slot: target });
    }),
    vscode.commands.registerCommand("terrain.findPath", async () => {
      const from = await pickNode(s, "Find path: from (type to search)");
      if (from === undefined) return;
      const to = await pickNode(s, "Find path: to (type to search)");
      if (to === undefined) return;
      await map.command({ type: "command", command: "path", from, to });
    }),
    vscode.commands.registerCommand("terrain.setLens", (lens: number) => map.command({ type: "command", command: "lens", lens: Number(lens) || 1 })),
    vscode.commands.registerCommand("terrain.blastRadius", () => map.command({ type: "command", command: "lens", lens: 6 })),
    vscode.commands.registerCommand("terrain.review", async () => {
      await map.command({ type: "command", command: "lens", lens: 6 });
      const item = await pickReviewFile(s);
      review.refresh(0);
      if (item) {
        await openNodeRef(root(), item.node);
        await map.reveal(item.node.slot);
      }
    }),
    vscode.commands.registerCommand("terrain.exportSvg", async () => {
      if (!MapPanel.current) {
        void vscode.window.showInformationMessage("Terrain: open the map (or a Neighbourhood) first; the export draws what it shows.");
        return;
      }
      await map.command({ type: "command", command: "exportSvg" });
    }),
    vscode.commands.registerCommand("terrain.showCycles", () => showCycles(s, map)),
    vscode.commands.registerCommand("terrain.showDeadCode", () => showDeadCode(s, map)),
    vscode.commands.registerCommand("terrain.showMisplaced", () => showMisplaced(s, map)),
    vscode.commands.registerCommand("terrain.showHiddenDependencies", () => showHidden(s, map)),
    vscode.workspace.onDidChangeConfiguration(async (e) => {
      // These change what is indexed: restart the indexer with them (the store keeps the work).
      if ((e.affectsConfiguration("terrain.exclude") || e.affectsConfiguration("terrain.history.enabled")) && s.serverInfo) {
        await map.restartIndexer();
        return;
      }
      if (!e.affectsConfiguration("terrain.map.capDataFiles") || !s.serverInfo) return;
      const client = await s.get();
      await client.request("settings/update", { capDataFiles: vscode.workspace.getConfiguration("terrain").get<boolean>("map.capDataFiles", true) });
    }),
  );
  return { session: s, map, watcher, context: contextView, lsp, review, activationMs: performance.now() - t0 };
}

/** The innermost node at the cursor (symbol, else file). */
async function slotAtCursor(s: Session): Promise<number | undefined> {
  const editor = vscode.window.activeTextEditor;
  const root = s.workspaceRoot;
  if (!editor || !root || editor.document.uri.scheme !== "file") return undefined;
  const rel = toRelative(root, editor.document.uri.fsPath);
  if (!rel) return undefined;
  const client = await s.get();
  const r = await client.request("node/locate", { path: rel, line: editor.selection.active.line + 1 });
  return r.node?.slot;
}

/** Quick pick backed by the indexer's search, re-queried as the user types. */
async function pickNode(s: Session, title: string): Promise<number | undefined> {
  const client = await s.get();
  const qp = vscode.window.createQuickPick<vscode.QuickPickItem & { slot: number }>();
  qp.title = title;
  qp.placeholder = "Symbol or file name";
  qp.matchOnDescription = true;
  let seq = 0;
  qp.onDidChangeValue(async (q) => {
    const mine = ++seq;
    if (!q.trim()) {
      qp.items = [];
      return;
    }
    qp.busy = true;
    const r = await client.request("search", { query: q, limit: 30 });
    if (mine !== seq) return;
    qp.items = r.hits.map((h) => ({ label: h.name, description: `${h.kind} · ${h.path}${h.kind === "file" ? "" : `:${h.line}`}`, slot: h.slot, alwaysShow: true }));
    qp.busy = false;
  });
  return new Promise((resolve) => {
    qp.onDidAccept(() => {
      resolve(qp.selectedItems[0]?.slot);
      qp.hide();
    });
    qp.onDidHide(() => {
      resolve(undefined);
      qp.dispose();
    });
    qp.show();
  });
}

/** Import cycles, largest first; picking one reveals it and names the edge that breaks it. */
async function showCycles(s: Session, map: MapController): Promise<void> {
  const client = await s.get();
  const r = await client.request("analysis/cycles", null);
  type Pick = vscode.QuickPickItem & { cycle?: CycleView };
  const describe = (c: CycleView): Pick => ({
    label: `${c.members.length} ${c.members[0]?.kind === "dir" ? "directories" : "files"}`,
    description: c.members.slice(0, 3).map((m) => m.path || m.name).join(", ") + (c.members.length > 3 ? ", ..." : ""),
    detail: c.breakFrom && c.breakTo ? `Breaking ${c.breakFrom.path || c.breakFrom.name} → ${c.breakTo.path || c.breakTo.name} removes the most of it` : undefined,
    cycle: c,
  });
  const items: Pick[] = [
    { label: "Files", kind: vscode.QuickPickItemKind.Separator },
    ...r.files.map(describe),
    { label: "Directories", kind: vscode.QuickPickItemKind.Separator },
    ...r.dirs.map(describe),
  ];
  if (!r.files.length && !r.dirs.length) {
    void vscode.window.showInformationMessage("Terrain: no import cycles found.");
    return;
  }
  const pick = await vscode.window.showQuickPick(items, { placeHolder: `${r.files.length} file cycles, ${r.dirs.length} directory cycles`, matchOnDescription: true, matchOnDetail: true });
  const first = pick?.cycle?.breakFrom ?? pick?.cycle?.members[0];
  if (first) await map.reveal(first.slot);
}

async function showDeadCode(s: Session, map: MapController): Promise<void> {
  const client = await s.get();
  const r = await client.request("analysis/dead", { limit: 2000 });
  type Pick = vscode.QuickPickItem & { ref: NodeRef };
  const items: Pick[] = r.items.map((ref) => ({ label: ref.name, description: `${ref.kind} · ${ref.path}:${ref.line}`, ref }));
  const pick = await vscode.window.showQuickPick(items, {
    placeHolder: `${r.total.toLocaleString("en-US")} dead-code candidates (nothing depends on them; not entry points or exported). Candidates, not verdicts.`,
    matchOnDescription: true,
  });
  if (pick) {
    await openNodeRef(s.workspaceRoot ?? "", pick.ref);
    if (MapPanel.current) await map.reveal(pick.ref.slot);
  }
}

/** Files whose module (from dependencies and co-change) lives mostly in another folder. */
async function showMisplaced(s: Session, map: MapController): Promise<void> {
  const client = await s.get();
  const r = await client.request("analysis/misplaced", { limit: 2000 });
  if (!r.items.length) {
    void vscode.window.showInformationMessage("Terrain: no misplaced files found.");
    return;
  }
  type Pick = vscode.QuickPickItem & { m: MisplacedView };
  const items: Pick[] = r.items.map((m) => ({
    label: m.node.name,
    description: m.node.path,
    detail: `Its module is ${Math.round(m.share * 100)}% in ${m.home.path || "/"}/`,
    m,
  }));
  const pick = await vscode.window.showQuickPick(items, {
    placeHolder: `${r.total.toLocaleString("en-US")} files whose module mostly lives in another folder`,
    matchOnDescription: true,
    matchOnDetail: true,
  });
  if (pick) {
    await openNodeRef(s.workspaceRoot ?? "", pick.m.node);
    if (MapPanel.current) await map.reveal(pick.m.node.slot);
  }
}

/** File pairs that change together in git but have no code link between them. */
async function showHidden(s: Session, map: MapController): Promise<void> {
  const client = await s.get();
  const r = await client.request("history/hidden", { limit: 2000 });
  if (!r.pairs.length) {
    const h = await client.request("history/summary", null);
    void vscode.window.showInformationMessage(
      h.available ? "Terrain: no hidden dependencies found (pairs with 5+ shared commits and no code link)." : "Terrain: no git history found for this workspace.",
    );
    return;
  }
  type Pick = vscode.QuickPickItem & { p: HiddenPair };
  const items: Pick[] = r.pairs.map((p) => ({
    label: `${p.a.name}  ↔  ${p.b.name}`,
    description: `${Math.round(p.degree * 100)}% · ${p.shared} shared commits`,
    detail: `${p.a.path}  ↔  ${p.b.path}`,
    p,
  }));
  const pick = await vscode.window.showQuickPick(items, {
    placeHolder: `${r.total.toLocaleString("en-US")} file pairs change together without a code link`,
    matchOnDescription: true,
    matchOnDetail: true,
  });
  if (pick) {
    await openNodeRef(s.workspaceRoot ?? "", pick.p.a);
    if (MapPanel.current) await map.reveal(pick.p.a.slot);
  }
}

export async function deactivate(): Promise<void> {
  session?.dispose();
}
