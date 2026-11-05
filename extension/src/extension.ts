// Entry point. Activation only registers commands and views; the indexer process starts
// lazily the first time something needs it.
import * as vscode from "vscode";
import { Session } from "./session.ts";
import { MapPanel } from "./panel.ts";

let session: Session | undefined;

export interface CompassApi {
  session: Session;
}

export function activate(context: vscode.ExtensionContext): CompassApi {
  const s = new Session(context);
  session = s;
  context.subscriptions.push(
    s,
    vscode.commands.registerCommand("compass.openMap", () => openMap(context, s)),
    vscode.commands.registerCommand("compass.reindex", async () => {
      await s.restart();
    }),
  );
  return { session: s };
}

async function openMap(context: vscode.ExtensionContext, s: Session): Promise<void> {
  const panel = MapPanel.show(context.extensionUri);
  panel.post({ type: "status", state: "starting", text: "Starting indexer" });
  try {
    await s.get();
    const v = s.serverInfo?.serverVersion ?? "?";
    panel.post({ type: "status", state: "ready", text: `Indexer ${v} connected` });
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    s.log.error(msg);
    panel.post({ type: "status", state: "error", text: msg });
  }
}

export async function deactivate(): Promise<void> {
  session?.dispose();
}
