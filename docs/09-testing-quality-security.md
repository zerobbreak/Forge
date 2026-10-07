# 09 — Testing, Quality, Performance, Security & Distribution

## Testing strategy

| Layer | Approach |
|---|---|
| Unit | Pure logic in `forge-core` against **fake** `GitBackend`, `ForgeHost`, `Store`, `SecretStore` |
| Git integration | **Real operations in temp repos** from fixtures: merge conflicts, detached HEAD, unborn branch, large files, CRLF, submodules, linked worktrees, bare repos |
| GitHub | Recorded HTTP fixtures or local mock server; a few opt-in live tests behind an env flag |
| TUI | Ratatui `TestBackend` snapshots (`insta`) at several sizes (incl. 80x24, ~100, ~70 cols); scripted key-sequence workflow tests |
| CLI | End-to-end tests asserting stdout, stderr, exit codes; **`--json` schema tests** |
| Safety | Prove destructive actions require confirmation; snapshots restore correctly |
| Compatibility | Compare `GitBackend` results with real `git` on the same fixtures |
| CI | Linux + macOS + Windows matrix: `cargo fmt`, `clippy -D warnings`, tests, release build |

Fixture rule: build fixtures programmatically (helper crate/module in `tests/fixtures`) rather than committing `.git` directories.

## Performance targets (initial, to be validated)

| Area | Target |
|---|---|
| Cold start → first render | < 150 ms in a typical repo |
| Status refresh (medium repo) | < 100 ms; incremental on FS events |
| Input latency | keypress → visible response < 50 ms, never blocked by I/O |
| Large repos | Virtualised lists, lazy diffs; usable with tens of thousands of files and long history |
| Memory | Modest, bounded caches; **never load entire history into memory** |
| Platforms | Linux, macOS, Windows (Windows Terminal + WSL) from day one; document tested terminals |
| Binary size | Tracked per release; avoid heavy deps in core |

Add benchmarks (e.g. `criterion`) for status, diff and log on a large fixture; run in CI as a trend check.

## Accessibility

Colour-blind-safe default theme · no meaning conveyed by colour alone · high-contrast theme · respect `NO_COLOR` · graceful glyph/colour degradation on limited terminals.

## Security & privacy

- Tokens only in **OS keyring** (or explicit env var). Never in SQLite, config, or logs.
- **No telemetry by default.** Any future analytics are opt-in and documented (D15).
- Shell-outs to `git` use **argument arrays**, never string-built commands. Treat repo paths and branch names as **untrusted input**.
- Repos may contain **malicious config, hooks or filters**: surface when Forge is about to execute a hook or external command.
- Release artefacts **checksummed and signed**; dependencies audited in CI (`cargo audit`, `cargo deny`).

## Distribution

- Prebuilt binaries via GitHub Releases: Linux (x86_64, aarch64), macOS (Intel + Apple Silicon), Windows. `cargo-dist` can automate.
- `cargo install forge-cli`, Homebrew tap, Scoop and winget manifests; distro packages later.
- Shell completions + man page generated from Clap.
- In-app update check **optional and off by default**; document manual upgrade path.

## Risk register

| Risk | Impact | Mitigation |
|---|---|---|
| libgit2 gaps (hooks, signing, some transports) | Behaviour differs from users' Git | Narrow documented CLI fallback behind `GitBackend`; compat tests vs real `git` |
| Data loss from a Forge bug | Destroys user trust | `SafetyService`, recovery snapshots, temp-repo tests, conservative defaults |
| Scope creep (rebase UI, issues, plugins) | No usable release | Strict MVP list; ADRs for additions; phase exit criteria |
| Cross-platform terminal quirks (esp. Windows) | Broken rendering/input | CI matrix, documented terminals, graceful degradation |
| Keyring unavailable on headless Linux | Login fails over SSH/containers | Clear error; env-var and `gh` token fallbacks; docs |
| GitHub API changes / rate limits | Stale or failing PR views | ETag caching, backoff, typed client with tests, small API surface |
| Large repo performance | Sluggish UI | Lazy loading, virtualised lists, CI benchmarks |
| Crowded market (lazygit, gitui) | Low adoption | Lead with GitHub integration, learnability, safety |
