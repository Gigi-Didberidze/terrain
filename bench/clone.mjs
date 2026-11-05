// Clones the benchmark corpus at pinned commits.
// Usage: node bench/clone.mjs [name ...] [--history]
//   --history  fetch full history (needed for git-mining benchmarks); default is depth 1.
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const { repos } = JSON.parse(readFileSync(join(here, "corpus.json"), "utf8"));
const args = process.argv.slice(2);
const history = args.includes("--history");
const wanted = args.filter((a) => !a.startsWith("--"));
const selected = wanted.length ? repos.filter((r) => wanted.includes(r.name)) : repos;
if (wanted.length && selected.length !== wanted.length) {
  console.error(`unknown repo; known: ${repos.map((r) => r.name).join(", ")}`);
  process.exit(2);
}

const git = (cwd, ...a) => execFileSync("git", a, { cwd, stdio: "inherit" });
const gitOut = (cwd, ...a) => execFileSync("git", a, { cwd, encoding: "utf8" }).trim();

for (const r of selected) {
  const dir = join(here, "corpus", r.name);
  if (existsSync(join(dir, ".git"))) {
    const head = gitOut(dir, "rev-parse", "HEAD");
    if (head === r.commit) {
      console.log(`${r.name}: already at ${r.commit.slice(0, 12)}`);
      continue;
    }
  } else {
    mkdirSync(dir, { recursive: true });
    git(dir, "init", "-q");
    git(dir, "remote", "add", "origin", r.url);
  }
  console.log(`${r.name}: fetching ${r.tag} (${r.commit.slice(0, 12)})${history ? " with history" : ""}`);
  const depth = history ? [] : ["--depth", "1"];
  git(dir, "-c", "core.longpaths=true", "fetch", "-q", ...depth, "origin", r.commit);
  git(dir, "-c", "core.longpaths=true", "-c", "advice.detachedHead=false", "checkout", "-q", "FETCH_HEAD");
}
