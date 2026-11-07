# Progress

**Current milestone:** M2 — Symbols and edges (gate: linux measurement pending)

## M0 — Skeleton: done
- Monorepo: Rust indexer workspace (`core`, `parse`, `git`, `engine`, `rpc`), extension host, webview, shared protocol schema with generated bindings.
- `Compass: Open Map` opens a themed panel backed by the indexer; verified by a VS Code integration test.
- CI: lint, typecheck, unit tests; per-OS build jobs (native and cross target each) producing six VSIX packages; integration tests on linux-x64, win32-x64, darwin-arm64; soft benchmark job.
- `bench/` clones the pinned corpus and writes `bench/results/latest.json` and `history.jsonl`.

## M1 — Files on a map: done (gate below)
Done:
- Discover (gitignore-aware parallel walk), hash (BLAKE3), LOC, generated-file detection, SQLite store with warm start.
- Deterministic squarified treemap with collapsed single-child chains, binary tiles, LOD culling, "+ N small files" aggregation.
- WebGL2 renderer (instanced rects, exact-size glyph atlas), Canvas2D fallback, pan/zoom, 150 ms transitions.
- Click to select, double-click to open, follow mode with "you are here" marker, fuzzy file search, hover card, legend, keyboard map.
- Incremental updates from the file watcher and saves.

### Measured (Windows 11, i5-1135G7 4C/8T, 8 GB RAM, Iris Xe, NVMe — below the plan's 8-core/16 GB reference machine)
| Metric | django | vscode | Budget |
| --- | --- | --- | --- |
| Files | 6,814 | 7,707 | |
| Cold index (files layer) | 464 ms | 295 ms | 15 s (1M LOC) |
| Map visible, warm start | 64 ms | 53 ms | 500 ms |
| Incremental save | 28 ms | 24 ms | 50 ms |
| Search p99 per keystroke | 2.3 ms | 2.0 ms | 30 ms |
| Indexer RSS | 22 MB | 25 MB | 300 MB |
| Pan/zoom (harness, D3D11) | 60.2 fps, max 16.8 ms, 0 > 32 ms | | 60 fps, none > 32 ms |

Notes:
- The very first read of a freshly cloned repo took 20 s (django) on Windows: antivirus scanning each file on first open. The bench scenario does one warm-up read before timing a cold index (see D7 for the same effect on the binary).

### M1 gate on linux v6.12 (86,661 files; 3 reserved-name files skipped on NTFS)
| Gate | Measured | Result |
| --- | --- | --- |
| Usable map within 10 s cold | first map 49–75 ms; all file nodes placed after discovery, 0.45–0.73 s (sizes estimated until read) | met |
| Map visible < 500 ms warm | 222–383 ms (6 runs after D16; first launch of a new binary 2 s, see D7) | met |
| 60 fps pan/zoom, no frame > 32 ms | 57.8–60.2 fps; 0–3 single missed vsyncs per 300 frames typical, one noisy run 11; draw CPU p90 5–7 ms | 60 fps met; strict max not met on this laptop (D18) |
| Save → updated map < 50 ms | server 11–23 ms; end to end incl. tile re-fetch 14–42 ms, median 16 ms | met |
| Cold full index < 4 min (files layer) | 98–308 s; bound by Windows Defender: plain Python read of the tree takes 217 s | environment-bound (D17); CI nightly measures on Linux |
| Indexer RSS | 90–110 MB | budget 1 GB |

Profiling log (measure first):
- Warm start 491–740 ms → profiled: SQLite rows 58–290 ms + graph insert 240–420 ms → snapshot blob, deferred search index, pre-layout, first-view priority (D16) → 222–383 ms.
- Linux pan/zoom 44.6 fps, CPU spikes 290 ms → profiled: global scene rebuild per arriving tile → per-tile batches with scissor (D15), time-sliced decoding, per-frame build budget, ASCII glyph prewarm (full 16 MB atlas re-upload per new glyph), label width cache and grid collisions → 58–60 fps.
- Cold hash 165–217 s → profiled per batch: scan only (merge 5–9 ms, persist 0 ms); 1 thread 8k files/s vs 8 threads 285 files/s on the same files; Python baseline 217 s → Defender (D17).

## M2 — Symbols and edges
Done:
- Tier 1 extraction for all 12 languages (TypeScript/JavaScript/TSX, Python, Go, Rust, Java, C, C++, C#, Kotlin, Ruby, PHP, Swift): one combined query walk per file (D20); Go methods and Rust/Swift impls/extensions attach to their types (D23).
- Tier 2 via the grammar's tags.scm (Lua, tested); Tier 3 path edges from Markdown, Dockerfile and configs.
- Import resolution per language (tsconfig paths/baseUrl, package.json workspaces, Python source roots, go.mod and vendor, Cargo crates and mod/use/super, Java/Kotlin packages, C/C++ include dirs, C# namespaces, Ruby require/require_relative, PHP PSR-4, Swift modules).
- Name resolution in four tiers (same file, imported, same package by directory or declared namespace, unique global at half weight); ambiguous names give no edge (D21, D27).
- Facts cached by content hash; incremental re-resolution of exactly the affected files; property test proves incremental == cold (and catches a deliberately removed step).
- Golden graphs for 12 fixture repos, each reviewed by hand.
- Map: symbols inside files on zoom; hover card with used-by/uses counts; bundled dependency edges on hover/selection (purple uses, orange used by, dashed = name-matched).

### M2 gate
| Gate | Measured | Result |
| --- | --- | --- |
| Golden tests for every Tier 1 fixture | 12/12 pass | met |
| Import resolution ≥ 95% vs language tooling (1,000 imports) | django 1000/1000 vs Python PathFinder (0 false edges); vscode 999/1000 vs TypeScript resolveModuleName (8 extras are CSS/JSON imports tsc doesn't model) | met |
| Cold index ~1M LOC < 15 s (with symbols and edges) | vscode 8.7 s (96k symbols, 201k calls, 52k imports), django 3.7–4.1 s; RSS 251 MB / 121–131 MB | met |
| Cold index linux < 4 min | in progress | |

Profiling log (measure first):
- vscode cold 13.7 s (budget 15 s) → per-file timing: extraction 4.2× slower in parallel than alone → mimalloc (D19): 13.2 s → per-query timing: four whole-tree walks per file → one combined query (D20): 8.7 s.
- Accuracy run found two real bugs: a same-length suffix match (django.utils.version → an unrelated __init__.py) and 67 false standard-library edges in Python (D22). Both fixed; regression tests added.

## Next
- Record the linux gate with symbols and edges; then M3: PageRank load-bearing score, fan-in/out, blast radius, Leiden communities, cycles, entry points, dead-code candidates, complexity; lenses 1 and 2; sidebar tree; CodeLens.

## Open questions
- None.
