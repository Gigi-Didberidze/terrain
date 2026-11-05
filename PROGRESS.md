# Progress

**Current milestone:** M2 — Symbols and edges (M1 gate recorded below)

## M0 — Skeleton: done
- Monorepo: Rust indexer workspace (`core`, `parse`, `git`, `engine`, `rpc`), extension host, webview, shared protocol schema with generated bindings.
- `Compass: Open Map` opens a themed panel backed by the indexer; verified by a VS Code integration test.
- CI: lint, typecheck, unit tests; per-OS build jobs (native and cross target each) producing six VSIX packages; integration tests on linux-x64, win32-x64, darwin-arm64; soft benchmark job.
- `bench/` clones the pinned corpus and writes `bench/results/latest.json` and `history.jsonl`.

## M1 — Files on a map: code complete, gate pending linux
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

## Next
- M2: tree-sitter extraction for Tier 1 languages (TypeScript, Python, Go, Rust first), import resolution, call/reference edges, golden fixtures, semantic zoom into symbols, hover counts, edge bundling.

## Open questions
- None.
