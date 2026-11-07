// Review mode: the status bar shows "Terrain: 14 changed · blast 212" when the branch differs
// from its base. Clicking it (or Terrain: Review Changes, Ctrl/Cmd+Alt+R) opens the map with
// the Change lens and lists the changed files by load-bearing score: what to read first.
import * as vscode from "vscode";
import type { ReviewItem, ReviewResult } from "../../../protocol/generated/protocol.ts";
import type { Session } from "../session.ts";

export class ReviewStatus implements vscode.Disposable {
  private readonly item: vscode.StatusBarItem;
  private timer: NodeJS.Timeout | undefined;
  private readonly session: Session;
  last: ReviewResult | undefined;

  constructor(session: Session) {
    this.session = session;
    this.item = vscode.window.createStatusBarItem("terrain.review", vscode.StatusBarAlignment.Left, 50);
    this.item.name = "Terrain review";
    this.item.command = "terrain.review";
  }

  /** Re-reads the change summary, debounced (the graph changes in bursts while editing). */
  refresh(delay = 1000): void {
    clearTimeout(this.timer);
    this.timer = setTimeout(() => void this.update(), delay);
  }

  private async update(): Promise<void> {
    if (!this.session.serverInfo) return;
    try {
      const client = await this.session.get();
      const r = await client.request("review/summary", { limit: 0 });
      this.last = r;
      if (!r.available || r.changedTotal === 0) {
        this.item.hide();
        return;
      }
      const n = r.changedTotal.toLocaleString("en-US");
      const b = r.blastTotal.toLocaleString("en-US");
      this.item.text = `$(git-compare) Terrain: ${n} changed · blast ${b}`;
      this.item.tooltip = `${n} files changed${r.baseName ? ` since this branch left ${r.baseName}` : ""} (committed and uncommitted); ${b} files depend on them.\nClick for the files to review first.`;
      this.item.accessibilityInformation = { label: `Terrain: ${n} files changed, ${b} files in the blast radius. Activate to review.` };
      this.item.show();
    } catch {
      this.item.hide();
    }
  }

  dispose(): void {
    clearTimeout(this.timer);
    this.item.dispose();
  }
}

/** The ranked list of changed files; picking one opens it. Returns the picked item. */
export async function pickReviewFile(session: Session): Promise<ReviewItem | undefined> {
  const client = await session.get();
  // Right after a branch switch the index may still be catching up; wait briefly.
  for (let i = 0; i < 100; i++) {
    const st = await client.request("index/status", null);
    if (st.phase === "idle" && st.files > 0) break;
    await new Promise((r) => setTimeout(r, 100));
  }
  await client.request("review/refresh", null);
  const r = await client.request("review/summary", { limit: 500 });
  if (!r.available) {
    void vscode.window.showInformationMessage("Terrain: no git repository with a base branch (main or master) found.");
    return undefined;
  }
  if (r.changedTotal === 0) {
    void vscode.window.showInformationMessage(`Terrain: nothing changed${r.baseName ? ` since ${r.baseName}` : ""}.`);
    return undefined;
  }
  type Pick = vscode.QuickPickItem & { item: ReviewItem };
  const items: Pick[] = r.changed.map((c) => ({
    label: c.node.name,
    description: c.node.path,
    detail: `${pct(c.load)} load-bearing · ${c.blastCount.toLocaleString("en-US")} depend on it${c.committed ? "" : " · uncommitted"}`,
    item: c,
  }));
  const offMap = r.pathsTotal - r.changedTotal;
  const pick = await vscode.window.showQuickPick(items, {
    title: `Review: ${r.changedTotal} changed${r.baseName ? ` vs ${r.baseName}` : ""}, ${r.blastTotal.toLocaleString("en-US")} files depend on them`,
    placeHolder: `Most load-bearing first: read these first${offMap > 0 ? ` (${offMap} deleted or unmapped files not listed)` : ""}`,
    matchOnDescription: true,
    matchOnDetail: true,
  });
  return pick?.item;
}

const pct = (load: number) => (load >= 0.5 ? `top ${Math.max(1, Math.round((1 - load) * 100))}%` : `bottom ${Math.max(1, Math.round(load * 100))}%`);
