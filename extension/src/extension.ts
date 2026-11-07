// Entry point. Activation only registers commands, views and listeners; the indexer process
// starts lazily the first time something needs it (budget: activation < 20 ms).
import * as vscode from "vscode";
import type { CycleView, HiddenPair, MisplacedView, NodeRef } from "../../protocol/generated/protocol.ts";
import { Session } from "./session.ts";
import { MapController } from "./mapController.ts";
import { Watcher } from "./watcher.ts";
import { MapPanel } from "./panel.ts";
import { ContextView, openNodeRef } from "./views/contextView.ts";
import { CompassCodeLens } from "./views/codeLens.ts";

let session: Session | undefined;

export interface CompassApi {
  session: Session;
  map: MapController;
  watcher: Watcher;
  context: ContextView;
  activationMs: number;
}

const CODE_LANGUAGES = ["typescript", "typescriptreact", "javascript", "javascriptreact", "python", "rust", "go", "java", "c", "cpp", "csharp", "kotlin", "ruby", "php", "swift", "lua"];

export function activate(context: vscode.ExtensionContext): CompassApi {
  const t0 = performance.now();
  const s = new Session(context);
  session = s;
  const map = new MapController(context, s);
  const watcher = new Watcher(s);
  const contextView = new ContextView(s);
  const lens = new CompassCodeLens(s);
  watcher.onHistory.event(() => contextView.historyChanged());
  map.onSelection.event((slot) => {
    if (slot !== null) void contextView.showSlot(slot);
  });
  s.onStart((client) => {
    client.on("map/changed", () => {
      contextView.refresh();
      lens.refresh();
    });
  });
  const root = () => s.workspaceRoot ?? "";
  context.subscriptions.push(
    s,
    map,
    watcher,
    contextView,
    lens,
    vscode.languages.registerCodeLensProvider(
      CODE_LANGUAGES.map((language) => ({ language, scheme: "file" })),
      lens,
    ),
    vscode.commands.registerCommand("compass.openMap", () => map.open()),
    vscode.commands.registerCommand("compass.reindex", async () => {
      await s.restart();
      MapPanel.current?.dispose();
      await map.open();
    }),
    vscode.commands.registerCommand("compass.searchMap", async () => {
      const panel = MapPanel.current ?? (await map.open());
      panel.post({ type: "command", command: "search" });
    }),
    vscode.commands.registerCommand("compass.openNodeRef", (r: NodeRef) => openNodeRef(root(), r)),
    vscode.commands.registerCommand("compass.revealNode", async (slot: number) => {
      await contextView.showSlot(slot);
      await vscode.commands.executeCommand("compass.context.focus");
      if (MapPanel.current) await map.reveal(slot);
    }),
    vscode.commands.registerCommand("compass.showCycles", () => showCycles(s, map)),
    vscode.commands.registerCommand("compass.showDeadCode", () => showDeadCode(s, map)),
    vscode.commands.registerCommand("compass.showMisplaced", () => showMisplaced(s, map)),
    vscode.commands.registerCommand("compass.showHiddenDependencies", () => showHidden(s, map)),
    vscode.workspace.onDidChangeConfiguration(async (e) => {
      if (!e.affectsConfiguration("compass.map.capDataFiles") || !s.serverInfo) return;
      const client = await s.get();
      await client.request("settings/update", { capDataFiles: vscode.workspace.getConfiguration("compass").get<boolean>("map.capDataFiles", true) });
    }),
  );
  return { session: s, map, watcher, context: contextView, activationMs: performance.now() - t0 };
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
    void vscode.window.showInformationMessage("Compass: no import cycles found.");
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
    void vscode.window.showInformationMessage("Compass: no misplaced files found.");
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
      h.available ? "Compass: no hidden dependencies found (pairs with 5+ shared commits and no code link)." : "Compass: no git history found for this workspace.",
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
