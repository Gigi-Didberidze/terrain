// Import-resolution accuracy for TypeScript/JavaScript against TypeScript's own module
// resolution (the resolver tsserver uses), with each file's nearest tsconfig.
// Usage: terrain-indexer imports <root> --lang typescript > imports.jsonl
//        node bench/accuracy/ts.mjs <root> imports.jsonl [--sample 1000]
import { createRequire } from "node:module";
import { readFileSync } from "node:fs";
import { dirname, join, relative, resolve, sep } from "node:path";

const require = createRequire(import.meta.url);
const ts = require("typescript"); // TypeScript 5 from bench/accuracy (the JS compiler API)

const [root, file] = process.argv.slice(2);
const sampleIdx = process.argv.indexOf("--sample");
const sampleSize = sampleIdx > 0 ? Number(process.argv[sampleIdx + 1]) : 1000;
const absRoot = resolve(root);
const rel = (p) => relative(absRoot, p).split(sep).join("/");

const configs = new Map();
function optionsFor(fromAbs) {
  const cfgPath = ts.findConfigFile(dirname(fromAbs), ts.sys.fileExists, "tsconfig.json") ?? ts.findConfigFile(dirname(fromAbs), ts.sys.fileExists, "jsconfig.json");
  if (!cfgPath) return { allowJs: true, moduleResolution: ts.ModuleResolutionKind.Bundler, module: ts.ModuleKind.ESNext };
  if (!configs.has(cfgPath)) {
    const cfg = ts.readConfigFile(cfgPath, ts.sys.readFile);
    const parsed = ts.parseJsonConfigFileContent(cfg.config ?? {}, ts.sys, dirname(cfgPath), undefined, cfgPath);
    configs.set(cfgPath, { allowJs: true, ...parsed.options });
  }
  return configs.get(cfgPath);
}

const rows = readFileSync(file, "utf8").trim().split("\n").map((l) => JSON.parse(l));
// Deterministic spread over the repo.
const stride = Math.max(1, Math.floor(rows.length / (sampleSize * 3)));
let checked = 0, agree = 0, externalButOurs = 0, externalTotal = 0;
const misses = [];
const falsePositives = [];
for (let i = 0; i < rows.length && checked < sampleSize; i += stride) {
  const r = rows[i];
  const fromAbs = join(absRoot, r.from);
  const res = ts.resolveModuleName(r.spec, fromAbs, optionsFor(fromAbs), ts.sys);
  const target = res.resolvedModule?.resolvedFileName;
  const inside = target && !target.includes("/node_modules/") && resolve(target).startsWith(absRoot);
  if (!inside) {
    externalTotal++;
    if (r.ours.length) {
      externalButOurs++;
      falsePositives.push({ from: r.from, spec: r.spec, oracle: target ?? null, ours: r.ours });
    }
    continue;
  }
  checked++;
  const expected = rel(target);
  if (r.ours.includes(expected)) agree++;
  else misses.push({ from: r.from, spec: r.spec, expected, ours: r.ours });
}
console.log(JSON.stringify({ lang: "typescript", checked, agree, rate: +(agree / checked).toFixed(4), oracleExternal: externalTotal, oursResolvedExternal: externalButOurs }, null, 1));
for (const m of misses.slice(0, 15)) console.log("MISS", JSON.stringify(m));
for (const m of falsePositives.slice(0, 15)) console.log("EXTRA", JSON.stringify(m));
