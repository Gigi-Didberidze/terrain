// Generates a synthetic polyglot monorepo for stress tests.
// Usage: node bench/synth.mjs <outDir> [--files 200000] [--seed 1]
// Files import each other within and across packages so the graph has real structure.
import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const args = process.argv.slice(2);
const out = args.find((a) => !a.startsWith("--"));
if (!out) {
  console.error("usage: node bench/synth.mjs <outDir> [--files N] [--seed S]");
  process.exit(2);
}
const opt = (name, dflt) => {
  const i = args.indexOf(`--${name}`);
  return i >= 0 ? Number(args[i + 1]) : dflt;
};
const total = opt("files", 200000);
let seed = opt("seed", 1) >>> 0;
const rand = () => ((seed = (seed * 1664525 + 1013904223) >>> 0) / 2 ** 32);
const pick = (n) => Math.floor(rand() * n);

const langs = [
  { ext: "ts", gen: (i, deps) => deps.map((d) => `import { f${d.i} } from "../${d.pkg}/m${d.i}";`).join("\n") + `\nexport function f${i}(x: number): number {\n  return ${deps.map((d) => `f${d.i}(x)`).join(" + ") || "x"};\n}\n` },
  { ext: "py", gen: (i, deps) => deps.map((d) => `from ${d.pkg}.m${d.i} import f${d.i}`).join("\n") + `\n\ndef f${i}(x):\n    return ${deps.map((d) => `f${d.i}(x)`).join(" + ") || "x"}\n` },
  { ext: "go", gen: (i) => `package p\n\nfunc F${i}(x int) int {\n\tif x > 0 {\n\t\treturn x\n\t}\n\treturn -x\n}\n` },
  { ext: "rs", gen: (i) => `pub fn f${i}(x: i64) -> i64 {\n    if x > 0 { x } else { -x }\n}\n` },
];

const packages = Math.max(1, Math.round(total / 400));
const perPkg = Math.ceil(total / packages);
let written = 0;
for (let p = 0; p < packages && written < total; p++) {
  const lang = langs[p % langs.length];
  const pkg = `pkg${p}`;
  const dir = join(out, lang.ext, pkg);
  mkdirSync(dir, { recursive: true });
  for (let i = 0; i < perPkg && written < total; i++, written++) {
    const deps = [];
    if (lang.ext === "ts" || lang.ext === "py") {
      const n = pick(4);
      for (let k = 0; k < n; k++) {
        // Mostly local deps, occasionally cross-package within the same language.
        const otherPkg = rand() < 0.8 ? p : (p + langs.length * (1 + pick(5))) % packages;
        const j = pick(perPkg);
        if (otherPkg === p && j === i) continue;
        deps.push({ pkg: `pkg${otherPkg}`, i: j });
      }
    }
    writeFileSync(join(dir, `m${i}.${lang.ext}`), lang.gen(i, deps));
  }
  if (lang.ext === "py") writeFileSync(join(dir, "__init__.py"), "");
}
console.log(`wrote ${written} files in ${packages} packages to ${out}`);
