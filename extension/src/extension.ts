// Entry point. Activation only registers commands and listeners; the indexer process starts
// lazily the first time something needs it (budget: activation < 20 ms).
import * as vscode from "vscode";
import { Session } from "./session.ts";
import { MapController } from "./mapController.ts";
import { Watcher } from "./watcher.ts";
import { MapPanel } from "./panel.ts";

let session: Session | undefined;

export interface CompassApi {
  session: Session;
  map: MapController;
  watcher: Watcher;
  activationMs: number;
}

export function activate(context: vscode.ExtensionContext): CompassApi {
  const t0 = performance.now();
  const s = new Session(context);
  session = s;
  const map = new MapController(context, s);
  const watcher = new Watcher(s);
  context.subscriptions.push(
    s,
    map,
    watcher,
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
  );
  return { session: s, map, watcher, activationMs: performance.now() - t0 };
}

export async function deactivate(): Promise<void> {
  session?.dispose();
}
