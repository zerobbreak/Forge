# 13 — Agent Playbook (How to Build Forge)

Operational instructions for an AI coding agent. Pair with `CLAUDE.md`. Everything here is derived from the spec; items marked *(suggested)* are implementation guidance, not spec requirements.

## 0. Session start routine

1. Read `CLAUDE.md`.
2. Open `docs/10-roadmap-and-mvp.md`; find the **current phase** (first phase with unticked exit criteria).
3. Read only the docs that phase touches (map in `CLAUDE.md`).
4. Check `docs/11-decisions.md` for Open/Proposed decisions the task depends on.
5. Plan a small vertical slice; write the test first where feasible.

## 1. Bootstrap *(suggested)*

```toml
# Cargo.toml (workspace root)
[workspace]
resolver = "2"
members = [
  "crates/forge-core", "crates/forge-git", "crates/forge-github",
  "crates/forge-storage", "crates/forge-tui", "crates/forge-cli",
]

[workspace.package]
edition = "2021"
license = "MIT OR Apache-2.0"   # pending licensing decision

[workspace.lints.clippy]
all = { level = "deny", priority = -1 }
```

- Binary name: `forge` (set `[[bin]] name = "forge"` in `forge-cli`; crate/package `forge-cli` for `cargo install`).
- `forge-core` `Cargo.toml` must NOT list `git2`, `reqwest`, `rusqlite`, `keyring`, `ratatui`, `crossterm`, `notify`. Add a CI check that fails if it does.
- Provide a `test-support` feature in `forge-core` exporting in-memory fakes.

## 2. Build order within a feature

1. Domain model + error variants in `forge-core`.
2. Trait method + fake implementation + service logic + unit tests.
3. Real adapter implementation + integration test on a temp repo.
4. CLI command (+ `--json` struct, contract test).
5. TUI view-model → widget → snapshot test.
6. Help content + error copy.
7. Event(s) emitted and consumed (activity recorder / logger).

## 3. Conventions *(suggested)*

- Errors: `thiserror` per layer; `anyhow` only in the binary's `main`. Never leak adapter error types through core APIs.
- No `unwrap()`/`expect()` in non-test code except for provably-infallible invariants with a comment.
- Time: store unix seconds (`i64`); inject a `Clock` trait for tests.
- Paths: canonicalise before storing; compare canonical forms.
- Secrets: newtype `Secret<String>` with redacting `Debug`; no `Display`.
- Public structs that cross crate or process boundaries derive `Serialize/Deserialize` and are versioned.
- Async: core service methods that hit the network are `async`; libgit2 calls are sync and wrapped with `spawn_blocking` at the dispatcher, not inside adapters.
- TUI: `fn update(state, event) -> state` style reducer; widgets are pure functions of view-model.
- Logging: `tracing` spans per command; field names stable.

## 4. Things agents commonly get wrong here — avoid

- Putting Git/HTTP/SQL code in the TUI or CLI crates "just for now".
- Writing TUI code that blocks on I/O (even a quick `git status`) on the UI thread.
- Forgetting to restore the terminal on panic/early-return paths.
- Mocking Git in integration tests instead of using temp repos.
- Scraping human-readable `git` output instead of porcelain/`-z` formats.
- Treating `libgit2` as running hooks or honouring `commit.gpgsign` — it does not.
- Silent plaintext token fallback when the keyring is missing.
- Silent decisions on Open ADRs.
- Implementing out-of-MVP features (rebase UI, stash UI, issues, Actions logs, plugins, Forge World).
- Making colour the only carrier of meaning (use glyphs/text as well).
- Breaking `--json` shapes without versioning.

## 5. Prompt templates for driving an agent

**Start a phase**
> Read `CLAUDE.md` and `docs/10-roadmap-and-mvp.md`. We are in Phase N. Implement the next unticked task from the Phase N checklist as a vertical slice following `docs/13-agent-playbook.md §2`. Include tests. Don't touch later phases. Summarise changes and any Open decisions you had to take.

**Implement a feature**
> Implement `<feature>` per `docs/<relevant>.md`. Add domain types and errors in `forge-core`, a fake + unit tests, the real adapter + a temp-repo integration test, the CLI command with `--json` contract test, and the TUI view with `TestBackend` snapshots at 80x24 and 120x40.

**Review a change**
> Review this diff against `CLAUDE.md` "Definition of done" and `docs/07-safety-and-errors.md`. Check layer boundaries, destructive-action safety, terminal restore, secret handling and error copy. List violations with file:line.

**Resolve an Open decision**
> Take decision D# from `docs/11-decisions.md`. Build a small spike with fixtures, record results in a new ADR (status Proposed), and recommend a choice with evidence.

## 6. Reference: Forge-owned artefacts

| Artefact | Location / name |
|---|---|
| Recovery snapshots | `refs/forge/snapshots/...` in the user's repo |
| SQLite DB | platform data dir (`directories`) |
| `config.toml` | platform config dir |
| Logs | platform state dir, rotated |
| Credential | OS keyring entry (service name `forge`) |

## 7. Glossary

- **Forge** — the product (binary `forge`). **`ForgeHost`** — trait abstracting the remote host (GitHub in v1).
- **Command** — a request from UI to core. **Event** — a fact emitted by core.
- **Snapshot (recovery)** — Forge-made ref preserving state before a destructive action. **Snapshot (repository)** — serialisable model of graph/refs/working-tree for plugins and Forge World. *Different things — name types accordingly (`RecoverySnapshot` vs `RepoSnapshot`).*
- **Composition root** — `forge-cli`'s `main`, where adapters are built and injected.
