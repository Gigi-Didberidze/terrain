// Launches a real VS Code with the extension under development and runs suite.cjs inside it.
import { runTests } from "@vscode/test-electron";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";

// Running from inside another Electron app (e.g. an editor terminal) leaks this and makes
// the test instance start as plain Node.
delete process.env.ELECTRON_RUN_AS_NODE;

const here = dirname(fileURLToPath(import.meta.url));
const workspace = mkdtempSync(resolve(tmpdir(), "compass-it-"));
writeFileSync(resolve(workspace, "hello.ts"), "export function hello() { return 1; }\n");
writeFileSync(resolve(workspace, "main.ts"), 'import { hello } from "./hello";\n\nexport function main() {\n  return hello();\n}\n');

try {
  await runTests({
    extensionDevelopmentPath: resolve(here, ".."),
    extensionTestsPath: resolve(here, "suite.cjs"),
    launchArgs: [workspace, "--disable-extensions", "--skip-welcome", "--skip-release-notes"],
  });
} catch (e) {
  console.error("integration tests failed", e);
  process.exit(1);
}
