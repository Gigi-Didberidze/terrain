# Progress

**Current milestone:** M5 — Neighbourhood, Path and Review (M4 done; M2's linux measurement deferred, see below)

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
| Cold index linux < 4 min, RSS < 1 GB | not measured: the run was stopped by the OS at ~66k of 86k files because the machine ran low on memory (8 GB shared with the editor and browser). Deferred by the user; to re-run with memory reporting | open |

Profiling log (measure first):
- vscode cold 13.7 s (budget 15 s) → per-file timing: extraction 4.2× slower in parallel than alone → mimalloc (D19): 13.2 s → per-query timing: four whole-tree walks per file → one combined query (D20): 8.7 s.
- Accuracy run found two real bugs: a same-length suffix match (django.utils.version → an unrelated __init__.py) and 67 false standard-library edges in Python (D22). Both fixed; regression tests added.

## M3 — Meaning: done (gate below)
- Load-bearing score: PageRank over the file dependency graph and the symbol graph (D29), shown on a log scale (D32).
- Fan-in/out in hover cards; blast radius (transitive dependents, depth ≤ 6) at symbol, file and directory level.
- Leiden communities on the file graph, deterministic (D33); 8 largest coloured.
- Import cycles at file and directory level (Tarjan), each with the edge whose removal breaks most of it (D31).
- Entry points per language; dead-code candidates (labelled candidates); cyclomatic complexity on demand.
- Lenses 1 (Structure) and 2 (Load-bearing); keys 3–6 say which milestone brings them.
- Context sidebar (native tree, follows the cursor or the map selection); CodeLens `N callers · blast N · N tests` (plus complexity when ≥ 10); Compass: Show Cycles / Show Dead-Code Candidates.
- Analysis runs after indexing and again 500 ms after edits settle, off the request path: django 38–71 ms, vscode 203 ms.

### M3 gate
| Gate | Result |
| --- | --- |
| django top 20 load-bearing files reviewed by hand | 19/20 judged core (below); utils/archive.py questionable |
| Cycle list matches dependency-cruiser on vscode `src/` | Compass finds all 384 files dependency-cruiser reports in cycles (100%); 6 extra files trace to one real 4-file loop (event → lifecycle → arrays → sequence → event) whose every edge dependency-cruiser itself reports but does not flag |

django top 20 (final run): 1 utils/datastructures · 2 utils/functional · 3 utils/connection · 4 utils/deprecation · 5 core/exceptions · 6 utils/hashable · 7 conf/__init__ · 8 utils/deconstruct · 9 db/models/__init__ · 10 utils/version · 11 utils/regex_helper · 12 utils/archive (questionable: likely name-matched `extract` calls) · 13 utils/translation/__init__ · 14 test/utils · 15 urls/__init__ · 16 db/__init__ · 17 utils/timezone · 18 utils/dateformat · 19 db/models/expressions · 20 django/__init__.

vscode top 20 (sanity check): base/common linkedList, lifecycle, uri, iterator, event, buffer, stream, arrays, skipList, resourceTree, prefixTree, dataTransfer; nls; a notebook test helper file (questionable); base/common/path; platform/instantiation; nls.messages; editor/common/core/position; base/common/async; base/common/platform.

Review log: the first ranking was wrong (template/loader_tags.py #1, a 486-file "cycle"), traced to the global name-matching tier; three rounds of fixes (D30, D31) produced the lists above.

## M4 — History: done (gate below)
- compass-git: history mined with gix (no `git` process), commit file lists diffed in parallel; incremental from the merge base of the old and new HEAD, so branch switches and rebases forget dropped commits (D37). Commits persist in the store; a restart re-reads them instead of re-mining.
- Churn (commits in the last 90 days), ownership by commits for the lens and by lines (blame) in the sidebar (D35), co-change coupling with support ≥ 5 (D36).
- Hidden dependencies: coupled pairs with no static edge, data/generated files left out, ranked by evidence. Co-change joins the Leiden graph. Misplaced files (D38).
- Lenses 3 (Churn), 4 (Coupling: partners of the selection, red = hidden, everything else muted) and 5 (Ownership, with an author key in the legend); hover cards show churn and owner.
- Sidebar: Churn, Owner, "Changes with" (partners, hidden ones marked). Commands: Show Misplaced Files, Show Hidden Dependencies. `.git/logs/HEAD` changes refresh history within ~1 s.

### M4 gate (vscode, full history: 126,385 commits, blobless clone)
| Gate | Result |
| --- | --- |
| History mined in < 60 s | 18.7 s mining, 19.9 s including statistics and the store write |
| Incremental update on new commits in < 1 s | 10 new commits: 485 ms end to end (mine 7 ms, statistics 267 ms, re-clustering with co-change); restart with nothing new: 561 ms |

Quality review: the first hidden-dependency list on vscode was led by test snapshots and translation files, and misplaced files included source modules "at home" in tests/ (on a private project) and files linked to a folder only through a cluster. After D38 and the evidence ranking, vscode's top pairs are dark/light theme files, generated .js next to their .ts, horizontal/vertical scrollbar, keybindingsSync/settingsSync; on the private project, JS files with their HTML templates, and the one misplaced file is used only from another package.

## Next
- M5: Neighbourhood view (ELK layered), Path view, LSP enrichment of visible nodes, Change lens, status-bar review entry, SVG export, full keyboard map.
- Pending: linux measurement with symbols and edges (memory).

## Open questions
- None.
