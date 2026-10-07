# 11 — Decision Register

Status: **Proposed** (recommendation, not final) · **Open** (undecided) · **Accepted**. Promote accepted rows to formal ADRs in `docs/adr/` (template: `docs/adr/0000-template.md`).

| ID | Decision | Options | Proposed choice & rationale | Revisit if | Status |
|---|---|---|---|---|---|
| D1 | Core language | Rust, Go, TypeScript | **Rust** — single native binary, strong TUI ecosystem, memory safety | Contributor pool/build times block | **Accepted** |
| D2 | Git access | git2 only, CLI only, hybrid | **Hybrid**: git2 default; CLI for hooks, signing, SSH, credential helpers | libgit2 closes gaps, or CLI parsing proves fragile | Proposed |
| D3 | GitHub auth type | OAuth App device flow, GitHub App w/ expiring tokens, PAT only | Evaluate **GitHub App** first (finer perms, expiring tokens); keep PAT + `gh` token fallbacks | Device-flow limits / org approval friction | Open |
| D4 | GitHub client | hand-rolled reqwest, Octocrab | Start **reqwest** + small typed surface; adopt Octocrab if upkeep gets costly | API surface grows past PRs + checks | Open |
| D5 | Local storage | SQLite, JSON files, embedded KV | **SQLite** — queryable activity + cache, proven, tiny | Never expected | **Accepted** |
| D6 | Secrets storage | OS keyring, encrypted file, plaintext | **OS keyring**; env-var fallback; **no silent plaintext** | Headless Linux demand justifies encrypted file | **Accepted** |
| D7 | Hook handling | ignore, always run, setting | **Run by default via CLI, setting to disable, surface when a hook executes** | Security concerns with untrusted repos | Open |
| D8 | Hunk/line staging | git2 apply to index, patch + `git apply` | Prototype **git2** first; fall back to patch generation if edge cases get unwieldy | Prototype fails fixture tests | Open |
| D9 | Undo/recovery | none, reflog hints, Forge snapshots | **Forge snapshot refs** before destructive local actions, pruned by age/count | Too costly on large/binary repos | Proposed |
| D10 | Concurrency | single-thread, thread pool, Tokio + blocking pool | **UI thread + Tokio I/O + bounded blocking pool for libgit2** | Profiling shows contention | Proposed |
| D11 | Plugin runtime | embedded Python, out-of-process JSON-RPC, WASM | **Out-of-process JSON-RPC** — crash isolation, no interpreter in core, permission boundary | Latency-sensitive plugins appear | Proposed |
| D12 | Plugin transport | stdio, local socket | **stdio** first | Need multiple concurrent clients | Open |
| D13 | Platform support v1 | Linux+macOS only, all 3 full, Windows beta | All three in CI from day one; decide if Windows ships beta | Terminal bugs consume schedule | Open |
| D14 | Host abstraction | GitHub-only code, trait | **`ForgeHost` trait now**, GitHub sole impl | Abstraction slows MVP | Proposed |
| D15 | Telemetry | none, opt-in, opt-out | **None by default**; any future analytics opt-in + documented | Need crash reporting at scale | Proposed |

## Additional open items (from the spec)

- [ ] Name collision check and licensing (MIT/Apache-2.0 dual vs other)
- [ ] Recovery snapshots: `refs/forge/` stash-style commits vs separate object store (links D9)
- [ ] Whether the integrated guide uses a sandbox repo in MVP or post-MVP

## How an agent should handle Open/Proposed decisions

1. Implement behind a trait, feature flag or setting so the choice is reversible.
2. Write `docs/adr/NNNN-title.md` with status **Proposed**, the option you chose, the evidence (fixture results, benchmarks) and the "revisit if" trigger.
3. State it explicitly in your change summary. Don't silently pick, don't block waiting.

## Decision lifecycle

```mermaid
stateDiagram-v2
    [*] --> Open: question raised
    Open --> Spike: needs evidence
    Spike --> Proposed: prototype + fixtures done
    Open --> Proposed: choice is clear
    Proposed --> Accepted: reviewed and agreed
    Proposed --> Open: new constraint found
    Accepted --> Superseded: later ADR replaces it
    Accepted --> [*]
    Superseded --> [*]
```
