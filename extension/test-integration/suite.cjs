// Runs inside the VS Code extension host. Plain assertions, no test framework.
const vscode = require("vscode");
const assert = require("node:assert/strict");

async function run() {
  const ext = vscode.extensions.all.find((e) => e.packageJSON.name === "terrain");
  assert.ok(ext, "extension is installed");
  const api = await ext.activate();
  const t0 = Date.now();
  await vscode.commands.executeCommand("terrain.openMap");
  const client = await api.session.get();
  const pong = await client.request("ping", null);
  assert.equal(typeof pong.uptimeMs, "number");
  assert.ok(api.session.serverInfo.serverVersion, "server reported a version");
  // The workspace's files must reach the map.
  let info;
  for (let i = 0; i < 100; i++) {
    info = await client.request("map/info", null);
    if (info.files >= 2 && info.phase === "idle") break;
    await new Promise((r) => setTimeout(r, 100));
  }
  // hello.ts, main.ts, shapes.ts, extra.ts, tsconfig.json
  assert.equal(info.files, 5, "every workspace file is on the map");
  const tiles = await client.request("map/tiles", { tiles: [{ z: 0, x: 0, y: 0 }] });
  assert.ok(Buffer.from(tiles.tiles[0].data, "base64").subarray(0, 4).toString() === "TRRN");
  assert.ok(api.activationMs < 20, `activation took ${api.activationMs} ms`);
  console.log(`[terrain-it] activated in ${api.activationMs.toFixed(1)} ms; map opened and indexer answered in ${Date.now() - t0} ms`);

  // M3: CodeLens above hello() reports its callers in main.ts and extra.ts.
  const folder = vscode.workspace.workspaceFolders[0].uri;
  const hello = vscode.Uri.joinPath(folder, "hello.ts");
  await vscode.window.showTextDocument(hello);
  let lenses = [];
  for (let i = 0; i < 50 && !lenses.some((l) => l.command); i++) {
    lenses = await vscode.commands.executeCommand("vscode.executeCodeLensProvider", hello);
    if (!lenses.some((l) => l.command)) await new Promise((r) => setTimeout(r, 200));
  }
  const titles = lenses.filter((l) => l.command).map((l) => l.command.title);
  assert.ok(titles.some((t) => t.startsWith("2 callers")), `CodeLens titles: ${JSON.stringify(titles)}`);

  // M3: the context of main() lists hello() among its callees.
  const loc = await client.request("node/locate", { path: "main.ts", line: 4 });
  const ctx = await client.request("node/context", { slot: loc.node.slot, limit: 10 });
  assert.ok(ctx.callees.some((c) => c.name === "hello"), `callees: ${JSON.stringify(ctx.callees)}`);
  console.log(`[terrain-it] CodeLens: ${titles.join(" | ")}`);

  // M5: Neighbourhood of main(): hello() and the name-matched Bag.has() on the right.
  const mainSlot = loc.node.slot;
  let nb = await client.request("view/neighbourhood", { slot: mainSlot });
  const names = (r) => r.nodes.filter((n) => n.column > 2).map((n) => n.node.name).sort();
  assert.deepEqual(names(nb), ["Bag.has", "hello"], `before enrichment: ${JSON.stringify(names(nb))}`);
  assert.ok(nb.millis < 100, `neighbourhood took ${nb.millis} ms`);

  // M5: the TypeScript language server confirms hello() and contradicts Bag.has().
  const mainUri = vscode.Uri.joinPath(folder, "main.ts");
  await vscode.window.showTextDocument(mainUri);
  for (let i = 0; i < 150; i++) {
    const items = await vscode.commands.executeCommand("vscode.prepareCallHierarchy", mainUri, new vscode.Position(2, 17));
    if (items && items.length) break;
    await new Promise((r) => setTimeout(r, 200));
  }
  api.lsp.enrich(mainSlot);
  for (let i = 0; i < 150 && api.lsp.stats.asked === 0; i++) await new Promise((r) => setTimeout(r, 100));
  assert.equal(api.lsp.stats.asked, 1, "call hierarchy answered");
  nb = await client.request("view/neighbourhood", { slot: mainSlot });
  assert.deepEqual(names(nb), ["hello"], `after enrichment: ${JSON.stringify(names(nb))}`);
  const toHello = nb.edges.find((e) => e.from === mainSlot);
  assert.equal(toHello.confidence, "semantic", "the confirmed call draws solid");
  console.log(`[terrain-it] language server confirmed ${api.lsp.stats.confirmed}, rejected ${api.lsp.stats.rejected}`);

  // M5: Path from extra.ts's file to hello.ts.
  const extra = await client.request("node/locate", { path: "extra.ts" });
  const helloFile = await client.request("node/locate", { path: "hello.ts" });
  const path = await client.request("view/path", { from: extra.node.slot, to: helloFile.node.slot });
  assert.deepEqual(path.paths.map((p) => p.nodes.map((n) => n.name)), [["extra.ts", "hello.ts"]]);

  // M5: review mode. On branch "feature": hello.ts changed in a commit, extra.ts is new.
  await client.request("review/refresh", null);
  const review = await client.request("review/summary", { limit: 10 });
  if (review.available) {
    assert.equal(review.baseName, "main");
    const changed = review.changed.map((c) => `${c.node.path}${c.committed ? "" : " (uncommitted)"}`).sort();
    assert.deepEqual(changed, ["extra.ts (uncommitted)", "hello.ts"]);
    // main.ts imports hello.ts: it is in the blast radius.
    assert.ok(review.blast.some((b) => b.path === "main.ts"), `blast: ${JSON.stringify(review.blast)}`);
    // The status bar may hold an earlier reading (taken before the review was computed); it
    // must catch up.
    api.review.refresh(0);
    for (let i = 0; i < 100 && api.review.last?.changedTotal !== 2; i++) await new Promise((r) => setTimeout(r, 100));
    assert.equal(api.review.last?.changedTotal, 2, "status bar shows the branch's changes");
    console.log(`[terrain-it] review: ${changed.join(", ")}; blast ${review.blastTotal}`);
  }

  // M5: commands run without error (Neighbourhood at the cursor, Change lens, SVG export).
  await vscode.window.showTextDocument(mainUri, { selection: new vscode.Range(3, 2, 3, 2) });
  await vscode.commands.executeCommand("terrain.showNeighbourhood");
  await vscode.commands.executeCommand("terrain.blastRadius");
}

module.exports = { run };
