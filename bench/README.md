# Benchmarks

- `node bench/clone.mjs [name ...] [--history]` clones the pinned corpus (`corpus.json`) into `bench/corpus/`.
- `node bench/run.mjs [--repos a,b] [--hard]` runs every scenario and checks `budgets.json`.
- `node bench/synth.mjs <dir> --files 200000` generates the synthetic polyglot monorepo.

On Windows, files named after reserved device names (`aux.c`, `con.h`, ...) can't exist on NTFS.
The clone uses a sparse checkout that leaves them out: 3 files in linux v6.12, none in the other repos.
