// Benchmark runner. Drives the real indexer binary over JSON-RPC, the same way the
// extension does, and checks results against bench/budgets.json.
//
// Usage: node bench/run.mjs [--repos linux,django] [--hard] [--binary path]
//   Repos not cloned under bench/corpus are skipped (clone with bench/clone.mjs).
//   Writes bench/results/latest.json and appends a line to bench/results/history.jsonl.
import { existsSync, mkdirSync, readFileSync, writeFileSync, appendFileSync, mkdtempSync, rmSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { tmpdir, cpus, totalmem } from "node:os";
import { execFileSync } from "node:child_process";
import { scenarios } from "./scenarios.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const args = process.argv.slice(2);
const flag = (n) => args.includes(`--${n}`);
const opt = (n) => {
  const i = args.indexOf(`--${n}`);
  return i >= 0 ? args[i + 1] : undefined;
};

const exe = process.platform === "win32" ? "terrain-indexer.exe" : "terrain-indexer";
const binary = opt("binary") ?? join(here, "..", "indexer", "target", "release", exe);
if (!existsSync(binary)) {
  console.error(`indexer binary not found at ${binary}; run: cargo build --release --manifest-path indexer/Cargo.toml`);
  process.exit(2);
}

const { repos } = JSON.parse(readFileSync(join(here, "corpus.json"), "utf8"));
const { budgets } = JSON.parse(readFileSync(join(here, "budgets.json"), "utf8"));
const wanted = opt("repos")?.split(",");
const targets = [];
// A tiny always-present target so the runner produces numbers even with no corpus cloned.
const fixture = join(here, "..", "fixtures");
if (!wanted || wanted.includes("fixtures")) targets.push({ name: "fixtures", root: fixture, size: "tiny" });
for (const r of repos) {
  if (wanted && !wanted.includes(r.name)) continue;
  const root = join(here, "corpus", r.name);
  if (existsSync(join(root, ".git"))) targets.push({ name: r.name, root, size: r.size });
  else console.log(`skip ${r.name}: not cloned (node bench/clone.mjs ${r.name})`);
}

const results = [];
for (const t of targets) {
  for (const s of scenarios) {
    const store = mkdtempSync(join(tmpdir(), "compass-bench-"));
    try {
      const metrics = await s.run({ binary, root: t.root, store, size: t.size });
      for (const [metric, value] of Object.entries(metrics)) {
        results.push({ repo: t.name, metric, value });
        console.log(`${t.name.padEnd(12)} ${metric.padEnd(36)} ${typeof value === "number" ? value.toFixed(1) : value}`);
      }
    } catch (e) {
      results.push({ repo: t.name, metric: `${s.name}.error`, value: String(e?.message ?? e) });
      console.log(`${t.name.padEnd(12)} ${s.name} FAILED: ${e?.message ?? e}`);
    } finally {
      rmSync(store, { recursive: true, force: true });
    }
  }
}

// Budget check.
const misses = [];
const sizeKey = { "1M": "1M", huge: "huge" };
for (const r of results) {
  if (typeof r.value !== "number") continue;
  const candidates = Object.entries(budgets).filter(([k]) => k === r.metric || k.startsWith(`${r.metric}.`));
  for (const [key, b] of candidates) {
    const suffix = key.slice(r.metric.length + 1);
    if (suffix && sizeKey[suffix] && targets.find((t) => t.name === r.repo)?.size !== suffix) continue;
    if (Array.isArray(b.repos) && !b.repos.includes(r.repo)) continue;
    if (r.value > b.max) misses.push({ ...r, budget: b.max, key });
  }
}

// Regressions: more than 10% worse than the previous run of the same metric on the same CPU
// model (comparing across machines would measure the hardware, not the code).
const regressions = [];
const cpuModel = cpus()[0]?.model;
const historyPath = join(here, "results", "history.jsonl");
if (existsSync(historyPath)) {
  const past = readFileSync(historyPath, "utf8").trim().split("\n").filter(Boolean).map((l) => JSON.parse(l)).filter((r) => r.machine?.cpuModel === cpuModel);
  const last = past[past.length - 1];
  if (last) {
    for (const r of results) {
      if (typeof r.value !== "number" || !/Ms$|MB$/.test(r.metric)) continue;
      const prev = last.results.find((p) => p.repo === r.repo && p.metric === r.metric);
      // Ignore sub-millisecond noise.
      if (prev && typeof prev.value === "number" && r.value > prev.value * 1.1 && r.value - prev.value > 2) {
        regressions.push({ ...r, previous: prev.value, commit: last.commit });
      }
    }
  }
}

let commit = "unknown";
try {
  commit = execFileSync("git", ["rev-parse", "--short", "HEAD"], { cwd: here, encoding: "utf8" }).trim();
} catch {}
const record = {
  date: new Date().toISOString(),
  commit,
  machine: { platform: process.platform, arch: process.arch, cpus: cpus().length, cpuModel: cpus()[0]?.model, memGB: Math.round(totalmem() / 2 ** 30) },
  results,
  misses,
  regressions,
};
mkdirSync(join(here, "results"), { recursive: true });
writeFileSync(join(here, "results", "latest.json"), JSON.stringify(record, null, 2) + "\n");
appendFileSync(join(here, "results", "history.jsonl"), JSON.stringify(record) + "\n");

if (misses.length) {
  console.log(`\n${misses.length} budget miss(es):`);
  for (const m of misses) console.log(`  ${m.repo} ${m.metric} = ${m.value.toFixed(1)} > ${m.budget} (${m.key})`);
}
if (regressions.length) {
  console.log(`\n${regressions.length} regression(s) over 10% vs ${regressions[0].commit}:`);
  for (const r of regressions) console.log(`  ${r.repo} ${r.metric} = ${r.value.toFixed(1)} (was ${r.previous.toFixed(1)})`);
}
const errors = results.filter((r) => r.metric.endsWith(".error"));
// From M2 on, budgets and regressions are hard gates in CI (--hard).
if ((flag("hard") && (misses.length || regressions.length)) || errors.length) process.exit(1);
