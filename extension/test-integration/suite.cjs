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
  // The workspace's one file must reach the map.
  let info;
  for (let i = 0; i < 100; i++) {
    info = await client.request("map/info", null);
    if (info.files >= 1 && info.phase === "idle") break;
    await new Promise((r) => setTimeout(r, 100));
  }
  assert.equal(info.files, 1, "hello.ts is on the map");
  const tiles = await client.request("map/tiles", { tiles: [{ z: 0, x: 0, y: 0 }] });
  assert.ok(Buffer.from(tiles.tiles[0].data, "base64").subarray(0, 4).toString() === "CMPT");
  assert.ok(api.activationMs < 20, `activation took ${api.activationMs} ms`);
  console.log(`[compass-it] map opened and indexer answered in ${Date.now() - t0} ms`);
}

module.exports = { run };
