// Compares Compass's import cycles with dependency-cruiser's on a TypeScript tree.
// Usage: terrain-indexer cycles <repo> > ours.jsonl
//        node bench/accuracy/cycles.mjs <repo> <subdir> ours.jsonl
// dependency-cruiser runs with the subdir's tsconfig and counts type-only imports, as Compass does.
import { execFileSync } from "node:child_process";
import { readFileSync, writeFileSync, mkdtempSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { tmpdir } from "node:os";

const [repo, sub, oursFile] = process.argv.slice(2);
const root = resolve(repo);
const cfg = join(mkdtempSync(join(tmpdir(), "depcruise-")), ".dependency-cruiser.cjs");
writeFileSync(
  cfg,
  `module.exports = {
  forbidden: [{ name: "no-circular", severity: "error", from: {}, to: { circular: true } }],
  options: {
    tsPreCompilationDeps: true,
    tsConfig: { fileName: ${JSON.stringify(join(root, sub, "tsconfig.json"))} },
    doNotFollow: { path: "node_modules" },
    exclude: { path: "node_modules" },
  },
};`,
);
// dependency-cruiser and TypeScript 5 are installed in bench/accuracy (npm install there).
const here = dirname(fileURLToPath(import.meta.url));
const bin = join(here, "node_modules", ".bin", "depcruise" + (process.platform === "win32" ? ".cmd" : ""));
const t0 = Date.now();
const out = execFileSync(bin, [sub, "--config", cfg, "--output-type", "json"], { cwd: root, maxBuffer: 1 << 30, encoding: "utf8", shell: process.platform === "win32" });
const result = JSON.parse(out);
const theirs = new Set();
for (const m of result.modules) {
  for (const d of m.dependencies) if (d.circular) theirs.add(m.source);
}
const ours = new Set();
for (const line of readFileSync(oursFile, "utf8").trim().split("\n").filter(Boolean)) {
  for (const f of JSON.parse(line).files) if (f.startsWith(sub + "/") && /\.(ts|tsx|js|mjs)$/.test(f)) ours.add(f);
}
const both = [...ours].filter((f) => theirs.has(f));
const onlyOurs = [...ours].filter((f) => !theirs.has(f));
const onlyTheirs = [...theirs].filter((f) => !ours.has(f));
console.log(
  JSON.stringify(
    {
      depcruiseSeconds: (Date.now() - t0) / 1000,
      filesInCycles: { compass: ours.size, depcruise: theirs.size, both: both.length },
      agreement: +(both.length / Math.max(1, new Set([...ours, ...theirs]).size)).toFixed(4),
    },
    null,
    1,
  ),
);
for (const f of onlyOurs.slice(0, 10)) console.log("ONLY-COMPASS", f);
for (const f of onlyTheirs.slice(0, 10)) console.log("ONLY-DEPCRUISE", f);
