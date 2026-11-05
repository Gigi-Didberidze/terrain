// Connects the map webview to the indexer: forwards whitelisted requests, converts tile
// payloads to raw bytes, opens files, and pushes "you are here" updates.
import * as vscode from "vscode";
import type { NodeInfo, TilesResult } from "../../protocol/generated/protocol.ts";
import { VIEW_REQUEST_METHODS, type ViewTile, type ViewToHost } from "../../protocol/webview.ts";
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

  constructor(context: vscode.ExtensionContext, session: Session) {
    this.context = context;
    this.session = session;
    session.onStart((client) => this.attach(client));
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
    panel.post({ type: "status", state: "starting", text: "Starting indexer" });
    try {
      await this.session.get();
      panel.post({ type: "status", state: "ready", text: "" });
      this.lastFollow = "";
      this.scheduleFollow(0);
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      this.session.log.error(msg);
      panel.post({ type: "status", state: "error", text: msg });
    }
    return panel;
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
      case "ready":
        return;
    }
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
