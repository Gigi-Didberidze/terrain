# Contributing

## Commit message structure

Every commit follows this exact structure. No exceptions, no extra trailers.

```
<type>(<scope>): <imperative summary, max 72 chars>

Milestone: <M0..M7 | none>
Changed:
- <path/to/file>: <function or item changed>
- <path/to/file>: <function or item changed>
Why: <one or two lines on the reason>
Verified: <tests / benchmark run, with numbers when relevant>
```

- `type`: `feat`, `fix`, `perf`, `test`, `docs`, `build`, `ci`, `refactor`, `chore`
- `scope`: `indexer`, `extension`, `webview`, `protocol`, `bench`, `ci`, `repo`
- Every commit names the files and the functions it changes (see the build plan's operating rule 4).
