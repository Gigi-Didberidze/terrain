// Connects the map webview to the indexer: forwards whitelisted requests, converts tile
// payloads to raw bytes, opens files, and pushes "you are here" updates.
import * as vscode from "vscode";
import type { NodeInfo, TilesResult } from "../../protocol/generated/protocol.ts";
import { STATE_COMMANDS, VIEW_REQUEST_METHODS, type HostToView, type ViewTile, type ViewToHost } from "../../protocol/webview.ts";
import type { IndexerClient } from "./indexerClient.ts";
import { MapPanel } from "./panel.ts";
import type { Session } from "./session.ts";
import { toRelative } from "./paths.ts";

export class MapController implements vscode.Disposable {
  private readonly disposables: vscode.Disposable[] = [];
  private followEnabled = true;
  private followTimer: NodeJS.Timeout | undefined;
  private lastFollow = "";
  /** Shared selection (map, sidebar and commands). */
  selection: number | null = null;
  readonly onSelection = new vscode.EventEmitter<number | null>();
  private readonly session: Session;
  private readonly context: vscode.ExtensionContext;
  /** Times of recent crashes: one is restarted quietly, repeated ones are shown. */
  private crashes: number[] = [];

  constructor(context: vscode.ExtensionContext, session: Session) {
    this.context = context;
    this.session = session;
    session.onStart((client) => this.attach(client));
    session.onCrash((code) => void this.onCrash(code));
    this.disposables.push(
      this.onSelection,
      vscode.window.onDidChangeActiveTextEditor(() => this.scheduleFollow()),
      vscode.window.onDidChangeTextEditorSelection(() => this.scheduleFollow()),
    );
  }

  private attach(client: IndexerClient): void {
    client.on("index/progress", (p) => MapPanel.current?.post({ type: "progress", phase: p.phase, done: p.done, total: p.total }));
    client.on("map/changed", (p) => MapPanel.current?.post({ type: "changed", epoch: p.epoch, layout: p.layout }));
  }

  async open(): Promise<MapPanel> {
    const panel = MapPanel.show(this.context.extensionUri);
    panel.onMessage = (m) => void this.onView(m);
    const followDefault = vscode.workspace.getConfiguration("compass").get<boolean>("map.follow", true);
    panel.post({ type: "config", followDefault });
    await this.connect(panel);
    return panel;
  }

  /** Starts (or reuses) the indexer and tells the view; failures become state screens. */
  private async connect(panel: MapPanel): Promise<void> {
    if (!this.session.workspaceRoot) {
      panel.post({
        type: "status",
        state: "empty",
        text: "Open a folder to see its map",
        detail: "Compass maps the folder you have open: its files, what they depend on, and how they change.",
        actions: [{ label: "Open Folder…", command: "workbench.action.files.openFolder" }],
      });
      return;
    }
    panel.post({ type: "status", state: "starting", text: "Starting the indexer" });
    try {
      await this.session.get();
      panel.post({ type: "status", state: "ready", text: "" });
      this.lastFollow = "";
      this.scheduleFollow(0);
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      this.session.log.error(msg);
      const missing = msg.includes("binary not found");
      panel.post({
        type: "status",
        state: "error",
        text: missing ? "Compass could not find its indexer" : "The indexer did not start",
        detail: missing
          ? "The compass-indexer program is missing from this installation. Reinstalling the extension restores it; the log lists where Compass looked."
          : `${msg.split("\n")[0]}. The log has the details.`,
        actions: missing
          ? [{ label: "Show Log", command: "compass.showLog" }]
          : [
              { label: "Restart Indexer", command: "compass.restartIndexer" },
              { label: "Show Log", command: "compass.showLog" },
            ],
      });
    }
  }

  /** The indexer stopped on its own: restart once quietly; a second crash within a minute is shown. */
  private async onCrash(code: number | null): Promise<void> {
    const now = Date.now();
    this.crashes = [...this.crashes.filter((t) => now - t < 60_000), now];
    const panel = MapPanel.current;
    if (this.crashes.length <= 1) {
      panel?.post({ type: "status", state: "starting", text: "The indexer stopped unexpectedly; restarting it" });
      if (panel) await this.connect(panel);
      else void this.session.get().catch(() => {});
      return;
    }
    panel?.post({
      type: "status",
      state: "error",
      text: "The indexer keeps stopping",
      detail: `It exited with code ${code ?? "unknown"} twice within a minute. The log usually says why; restarting tries once more.`,
      actions: [
        { label: "Restart Indexer", command: "compass.restartIndexer" },
        { label: "Show Log", command: "compass.showLog" },
      ],
    });
  }

  /** Restarts the indexer and reconnects the open map. */
  async restartIndexer(): Promise<void> {
    this.crashes = [];
    await this.session.restart().catch(() => {});
    const panel = MapPanel.current;
    if (panel) await this.connect(panel);
  }

  private async onView(m: ViewToHost): Promise<void> {
    const panel = MapPanel.current;
    if (!panel) return;
    switch (m.type) {
      case "request": {
        if (!VIEW_REQUEST_METHODS.includes(m.method)) {
          panel.post({ type: "response", id: m.id, error: `method not allowed: ${m.method}` });
          return;
        }
        try {
          const client = await this.session.get();
          let result: unknown = await client.request(m.method, m.params as never);
          if (m.method === "map/tiles") result = toViewTiles(result as TilesResult);
          panel.post({ type: "response", id: m.id, result });
        } catch (e) {
          panel.post({ type: "response", id: m.id, error: e instanceof Error ? e.message : String(e) });
        }
        return;
      }
      case "open":
        await this.openNode(m.slot);
        return;
      case "select":
        this.selection = m.slot;
        this.onSelection.fire(m.slot);
        return;
      case "follow":
        this.followEnabled = m.enabled;
        return;
      case "log":
        if (m.level === "error") this.session.log.error(m.text);
        else this.session.log.info(m.text);
        return;
      case "svg":
        await this.saveSvg(m.svg);
        return;
      case "action":
        if (STATE_COMMANDS.includes(m.command)) await vscode.commands.executeCommand(m.command);
        return;
      case "ready":
        return;
    }
  }

  /** Sends a command to the map, opening it first if needed. */
  async command(msg: HostToView): Promise<void> {
    // show() brings an existing panel to the front; open() also starts the indexer.
    const panel = MapPanel.current ? MapPanel.show(this.context.extensionUri) : await this.open();
    panel.post(msg);
  }

  private async saveSvg(svg: string): Promise<void> {
    const root = this.session.workspaceRoot;
    const target = await vscode.window.showSaveDialog({
      defaultUri: root ? vscode.Uri.joinPath(vscode.Uri.file(root), "compass-view.svg") : undefined,
      filters: { SVG: ["svg"] },
      saveLabel: "Export",
    });
    if (!target) return;
    await vscode.workspace.fs.writeFile(target, Buffer.from(svg, "utf8"));
    void vscode.window.showInformationMessage(`Compass: view exported to ${vscode.workspace.asRelativePath(target)}`);
  }

  async openNode(slot: number): Promise<void> {
    const root = this.session.workspaceRoot;
    if (!root) return;
    const client = await this.session.get();
    const n = await client.request("node/info", { slot });
    if (n.kind === "dir") return;
    const uri = vscode.Uri.joinPath(vscode.Uri.file(root), n.path);
    const line = Math.max(0, (n.kind === "file" ? 1 : n.startLine) - 1);
    const pos = new vscode.Position(line, 0);
    await vscode.window.showTextDocument(uri, { selection: new vscode.Range(pos, pos), preview: false, viewColumn: vscode.ViewColumn.One });
  }

  /** Reveals a node on the map (from the sidebar, commands or search). */
  async reveal(slot: number): Promise<void> {
    const panel = MapPanel.current ?? (await this.open());
    panel.post({ type: "reveal", slot });
  }

  /** Follow mode: the map tracks the active editor, debounced 250 ms. */
  private scheduleFollow(delay = 250): void {
    if (!MapPanel.current || !this.session.serverInfo) return;
    clearTimeout(this.followTimer);
    this.followTimer = setTimeout(() => void this.pushFollow(), delay);
  }

  private async pushFollow(): Promise<void> {
    const panel = MapPanel.current;
    const editor = vscode.window.activeTextEditor;
    const root = this.session.workspaceRoot;
    if (!panel || !editor || !root || editor.document.uri.scheme !== "file") return;
    const rel = toRelative(root, editor.document.uri.fsPath);
    if (rel === undefined) return;
    const line = editor.selection.active.line + 1;
    try {
      const client = await this.session.get();
      const r = await client.request("node/locate", { path: rel, line });
      const node: NodeInfo | null = r.node ?? null;
      const key = `${node?.slot ?? -1}`;
      if (key === this.lastFollow) return;
      this.lastFollow = key;
      if (this.followEnabled || node === null) panel.post({ type: "follow", node });
      else panel.post({ type: "follow", node: { ...node, rect: undefined } });
    } catch {
      // Indexer restarting.
    }
  }

  dispose(): void {
    clearTimeout(this.followTimer);
    for (const d of this.disposables.splice(0)) d.dispose();
  }
}

function toViewTiles(r: TilesResult): { epoch: number; tiles: ViewTile[] } {
  return {
    epoch: r.epoch,
    tiles: r.tiles.map((t) => ({ z: t.z, x: t.x, y: t.y, data: new Uint8Array(Buffer.from(t.data, "base64")) })),
  };
}
