// Launches a real VS Code with the extension under development and runs suite.cjs inside it.
import { runTests } from "@vscode/test-electron";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { appendFileSync, mkdtempSync, writeFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { tmpdir } from "node:os";

// Running from inside another Electron app (e.g. an editor terminal) leaks this and makes
// the test instance start as plain Node.
delete process.env.ELECTRON_RUN_AS_NODE;

const here = dirname(fileURLToPath(import.meta.url));
const workspace = mkdtempSync(resolve(tmpdir(), "terrain-it-"));
writeFileSync(resolve(workspace, "hello.ts"), "export function hello() { return 1; }\n");
// main() calls Set.has(); Terrain's name matching also links it to Bag.has() in the same
// folder, which the TypeScript language server must contradict (M5 semantic enrichment).
writeFileSync(resolve(workspace, "shapes.ts"), "export class Bag {\n  has(x: number) {\n    return x > 0;\n  }\n}\n");
writeFileSync(
  resolve(workspace, "main.ts"),
  'import { hello } from "./hello";\n\nexport function main() {\n  const seen = new Set<number>();\n  return hello() + (seen.has(1) ? 1 : 0);\n}\n',
);
writeFileSync(resolve(workspace, "tsconfig.json"), '{ "compilerOptions": { "strict": true, "target": "es2022" } }\n');
// A git history with a feature branch: one committed change and one new untracked file (M5 review).
const git = (...args) => execFileSync("git", args, { cwd: workspace, stdio: "pipe" });
try {
  git("init", "-q", "-b", "main");
  git("config", "user.email", "it@example.com");
  git("config", "user.name", "IT");
  git("add", "-A");
  git("commit", "-q", "-m", "init");
  git("checkout", "-q", "-b", "feature");
  appendFileSync(resolve(workspace, "hello.ts"), "export function bye() { return 2; }\n");
  git("commit", "-q", "-am", "bye");
  writeFileSync(resolve(workspace, "extra.ts"), 'import { hello } from "./hello";\nexport const x = hello();\n');
} catch (e) {
  console.warn("git not available; review checks will be skipped", e.message);
}

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
