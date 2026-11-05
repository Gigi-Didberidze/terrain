// The Compass map panel: a single webview that renders the map. All data comes from the
// indexer via the extension host; the webview never talks to the indexer directly.
import * as vscode from "vscode";
import { randomBytes } from "node:crypto";
import type { HostToView, ViewToHost } from "../../protocol/webview.ts";

export class MapPanel implements vscode.Disposable {
  static current: MapPanel | undefined;
  private readonly panel: vscode.WebviewPanel;
  private readonly disposables: vscode.Disposable[] = [];
  private viewReady = false;
  private readonly queue: HostToView[] = [];
  onMessage: ((m: ViewToHost) => void) | undefined;

  static show(extensionUri: vscode.Uri): MapPanel {
    if (MapPanel.current) {
      MapPanel.current.panel.reveal();
      return MapPanel.current;
    }
    const panel = vscode.window.createWebviewPanel("compass.map", "Compass", vscode.ViewColumn.Active, {
      enableScripts: true,
      retainContextWhenHidden: true,
      localResourceRoots: [vscode.Uri.joinPath(extensionUri, "dist", "webview")],
    });
    MapPanel.current = new MapPanel(panel, extensionUri);
    return MapPanel.current;
  }

  private constructor(panel: vscode.WebviewPanel, extensionUri: vscode.Uri) {
    this.panel = panel;
    panel.webview.html = renderHtml(panel.webview, extensionUri);
    panel.onDidDispose(() => this.dispose(), null, this.disposables);
    panel.webview.onDidReceiveMessage(
      (m: ViewToHost) => {
        if (m.type === "ready") {
          this.viewReady = true;
          for (const q of this.queue.splice(0)) void panel.webview.postMessage(q);
        }
        this.onMessage?.(m);
      },
      null,
      this.disposables,
    );
  }

  post(msg: HostToView): void {
    if (!this.viewReady) this.queue.push(msg);
    else void this.panel.webview.postMessage(msg);
  }

  dispose(): void {
    MapPanel.current = undefined;
    for (const d of this.disposables.splice(0)) d.dispose();
    this.panel.dispose();
  }
}

export function renderHtml(webview: vscode.Webview, extensionUri: vscode.Uri): string {
  const nonce = randomBytes(16).toString("base64");
  const base = vscode.Uri.joinPath(extensionUri, "dist", "webview");
  const script = webview.asWebviewUri(vscode.Uri.joinPath(base, "main.js"));
  const style = webview.asWebviewUri(vscode.Uri.joinPath(base, "main.css"));
  const csp = [
    "default-src 'none'",
    `style-src ${webview.cspSource}`,
    `script-src 'nonce-${nonce}' ${webview.cspSource}`,
    `font-src ${webview.cspSource}`,
    `img-src ${webview.cspSource} data: blob:`,
    "worker-src blob:",
  ].join("; ");
  return `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta http-equiv="Content-Security-Policy" content="${csp}">
<meta name="viewport" content="width=device-width, initial-scale=1">
<link rel="stylesheet" href="${style}">
<title>Compass</title>
</head>
<body>
<div id="app" role="application" aria-label="Compass code map"></div>
<script type="module" nonce="${nonce}" src="${script}"></script>
</body>
</html>`;
}
