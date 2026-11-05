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
  console.log(`[compass-it] map opened and indexer answered in ${Date.now() - t0} ms`);
}

module.exports = { run };
