// Runs inside the VS Code extension host. Plain assertions, no test framework.
const vscode = require("vscode");
const assert = require("node:assert/strict");

async function run() {
  const ext = vscode.extensions.all.find((e) => e.packageJSON.name === "compass-map");
  assert.ok(ext, "extension is installed");
  const api = await ext.activate();
  const t0 = Date.now();
  await vscode.commands.executeCommand("compass.openMap");
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
  assert.equal(info.files, 2, "hello.ts and main.ts are on the map");
  const tiles = await client.request("map/tiles", { tiles: [{ z: 0, x: 0, y: 0 }] });
  assert.ok(Buffer.from(tiles.tiles[0].data, "base64").subarray(0, 4).toString() === "CMPT");
  assert.ok(api.activationMs < 20, `activation took ${api.activationMs} ms`);
  console.log(`[compass-it] map opened and indexer answered in ${Date.now() - t0} ms`);

  // M3: CodeLens above hello() reports its caller in main.ts.
  const folder = vscode.workspace.workspaceFolders[0].uri;
  const hello = vscode.Uri.joinPath(folder, "hello.ts");
  await vscode.window.showTextDocument(hello);
  let lenses = [];
  for (let i = 0; i < 50 && !lenses.some((l) => l.command); i++) {
    lenses = await vscode.commands.executeCommand("vscode.executeCodeLensProvider", hello);
    if (!lenses.some((l) => l.command)) await new Promise((r) => setTimeout(r, 200));
  }
  const titles = lenses.filter((l) => l.command).map((l) => l.command.title);
  assert.ok(titles.some((t) => t.startsWith("1 caller")), `CodeLens titles: ${JSON.stringify(titles)}`);

  // M3: the context of main() lists hello() among its callees.
  const loc = await client.request("node/locate", { path: "main.ts", line: 4 });
  const ctx = await client.request("node/context", { slot: loc.node.slot, limit: 10 });
  assert.ok(ctx.callees.some((c) => c.name === "hello"), `callees: ${JSON.stringify(ctx.callees)}`);
  console.log(`[compass-it] CodeLens: ${titles.join(" | ")}`);
}

module.exports = { run };
