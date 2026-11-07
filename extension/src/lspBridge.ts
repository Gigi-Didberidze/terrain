// Semantic enrichment: for the symbol the developer is looking at (sidebar, map selection,
// Neighbourhood centre), ask VS Code's own call hierarchy (each language's server) for its
// callers and callees, and send the answer to the indexer, which marks those calls confirmed
// and drops the name-matched guesses the language server contradicts. Terrain never starts a
// language server of its own; without one, nothing happens.
import * as vscode from "vscode";
import type { NodeInfo, SourceLocation } from "../../protocol/generated/protocol.ts";
import type { Session } from "./session.ts";
import { toRelative } from "./paths.ts";

/** Kinds call hierarchy answers for. */
const CALLABLE = new Set(["function", "method", "class"]);
/** At most this many call-hierarchy requests run at once. */
const CONCURRENCY = 2;

export class LspBridge implements vscode.Disposable {
  private readonly session: Session;
  /** Slots already confirmed for a given graph epoch (re-asked after edits). */
  private readonly done = new Map<number, number>();
  private readonly queue: number[] = [];
  private running = 0;
  /** Confirmed / rejected totals, for the log and tests. */
  readonly stats = { asked: 0, confirmed: 0, rejected: 0 };

  constructor(session: Session) {
    this.session = session;
  }

  /** Asks for `slot` soon (deduplicated, bounded concurrency). */
  enrich(slot: number): void {
    if (this.queue.includes(slot)) return;
    this.queue.push(slot);
    if (this.queue.length > 20) this.queue.shift();
    this.pump();
  }

  private pump(): void {
    while (this.running < CONCURRENCY && this.queue.length) {
      const slot = this.queue.shift()!;
      this.running++;
      void this.confirm(slot).finally(() => {
        this.running--;
        this.pump();
      });
    }
  }

  private async confirm(slot: number): Promise<void> {
    const root = this.session.workspaceRoot;
    if (!root || !vscode.workspace.getConfiguration("terrain").get<boolean>("semanticEnrichment.enabled", true)) return;
    try {
      const client = await this.session.get();
      const n: NodeInfo = await client.request("node/info", { slot });
      if (!CALLABLE.has(n.kind)) return;
      // A file's content change forgets its confirmations; the epoch tells us when to re-ask.
      const key = (await client.request("index/status", null)).epoch;
      if (this.done.get(slot) === key) return;
      const uri = vscode.Uri.joinPath(vscode.Uri.file(root), n.path);
      const doc = await vscode.workspace.openTextDocument(uri);
      const line = Math.min(doc.lineCount - 1, Math.max(0, n.startLine - 1));
      const col = doc.lineAt(line).text.indexOf(n.name);
      if (col < 0) return;
      const pos = new vscode.Position(line, col);
      const items = await vscode.commands.executeCommand<vscode.CallHierarchyItem[] | undefined>("vscode.prepareCallHierarchy", uri, pos);
      const item = items?.[0];
      if (!item) return;
      const [incoming, outgoing] = await Promise.all([
        vscode.commands.executeCommand<vscode.CallHierarchyIncomingCall[] | undefined>("vscode.provideIncomingCalls", item),
        vscode.commands.executeCommand<vscode.CallHierarchyOutgoingCall[] | undefined>("vscode.provideOutgoingCalls", item),
      ]);
      const loc = (i: vscode.CallHierarchyItem): SourceLocation | undefined => {
        if (i.uri.scheme !== "file") return undefined;
        const path = toRelative(root, i.uri.fsPath);
        return path ? { path, line: i.selectionRange.start.line + 1 } : undefined;
      };
      const callers = (incoming ?? []).map((c) => loc(c.from)).filter((l): l is SourceLocation => !!l);
      const callees = (outgoing ?? []).map((c) => loc(c.to)).filter((l): l is SourceLocation => !!l);
      const r = await client.request("edges/confirm", { slot, callers, callees });
      this.stats.asked++;
      this.stats.confirmed += r.confirmed;
      this.stats.rejected += r.rejected;
      this.done.set(slot, (await client.request("index/status", null)).epoch);
      if (r.confirmed || r.rejected) {
        this.session.log.info(`confirmed ${r.confirmed} and rejected ${r.rejected} calls of ${n.qualified} via the language server${r.unmatched ? ` (${r.unmatched} locations not on the map)` : ""}`);
      }
    } catch (e) {
      this.session.log.warn(`call hierarchy for slot ${slot} failed: ${e instanceof Error ? e.message : e}`);
    }
  }

  dispose(): void {
    this.queue.length = 0;
  }
}
