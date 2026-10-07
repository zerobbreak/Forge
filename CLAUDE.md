# CLAUDE.md — Forge

> Entry point for any AI coding agent working on Forge. Read this first, then open the docs it points to **only when the task needs them**. This file is also valid as `AGENTS.md` (copy or symlink it).

## What Forge is

**Forge** is a terminal-native Git + GitHub development environment: **one Rust binary (`forge`), two interaction styles (TUI and CLI), no backend.**

One-liner: *Git and GitHub, made visible and learnable, without leaving the terminal.*

Forge does **not** replace Git. Git is the source of truth; Forge reads `.git` and calls the GitHub API, and presents both through one discoverable, safe interface.

## Non-negotiable principles

1. **Git is the source of truth.** Never copy or shadow history. Read `.git`, call GitHub.
2. **Same core, two front ends.** Every capability is a service in `forge-core`. CLI and TUI are thin shells. No Git, HTTP or SQL in presentation code.
3. **Safe by default.** Destructive actions are previewed, confirmed and (where possible) recoverable. See `docs/07-safety-and-errors.md`.
4. **Teach as you go.** Contextual footer, `?` overlay, guide, plain-language errors. All help text comes from ONE shared content source.
5. **Small footprint.** Single native binary, local SQLite, OS keychain, HTTPS. Nothing to host.
6. **Never block input.** Long operations run in the background with progress and `Esc` cancellation.
7. **Local vs remote is always labelled.** Anything from GitHub carries a remote marker and freshness timestamp.

## Workspace layout (Cargo workspace)

```
forge/
├── crates/
│   ├── forge-core/      # services, domain models, traits, events, errors. NO I/O deps.
│   ├── forge-git/       # GitBackend impl (git2 + narrow system-git fallback)
│   ├── forge-github/    # ForgeHost impl, device-flow auth, caching helpers
│   ├── forge-storage/   # SQLite (rusqlite), migrations, config.toml loading
│   ├── forge-tui/       # ratatui views, widgets, keymap, view-models
│   └── forge-cli/       # clap commands + binary entry point (composition root)
├── plugins/python/      # LATER — do not build in MVP
├── tests/{integration,fixtures}/
├── docs/                # guide content, ADRs
├── Cargo.toml  README.md  LICENSE
```

**Dependency rule:** `presentation → core → adapters (via traits)`. Adapters never depend on presentation. `forge-core` depends only on traits (`GitBackend`, `ForgeHost`, `Store`, `SecretStore`) so it is testable with fakes. `forge-cli` is the **composition root**: it builds concrete adapters and injects them.

## Tech stack (fixed unless an ADR changes it)

Rust · Ratatui + Crossterm · Clap · git2 (libgit2) + system `git` fallback · Tokio + Reqwest (Octocrab optional) · Serde · rusqlite · `keyring` · `directories` + TOML · `tracing` · `notify` · tests: `tempfile`, Ratatui `TestBackend`, `insta`.

Rule: *if Forge needs it to function, it is Rust; if it is optional user extensibility, it is Python (post-MVP).*

## Documentation map

| Need | Read |
|---|---|
| Why / who / scope | `docs/01-product-vision.md` |
| Crates, layers, services, traits | `docs/02-architecture.md` |
| git2 vs git CLI per operation | `docs/03-git-engine.md` |
| Auth, API, caching, offline | `docs/04-github-integration.md` |
| SQLite schema, config, paths | `docs/05-data-and-storage.md` |
| Threads, channels, events | `docs/06-events-and-concurrency.md` |
| Risk levels, snapshots, errors, logging | `docs/07-safety-and-errors.md` |
| Commands, JSON contracts, TUI layout, keymap | `docs/08-cli-and-tui-spec.md` |
| Tests, CI, perf, a11y, security | `docs/09-testing-quality-security.md` |
| Phases, tasks, MVP acceptance | `docs/10-roadmap-and-mvp.md` |
| Decision register & open questions | `docs/11-decisions.md` |
| Plugins & Forge World (post-MVP constraints) | `docs/12-future-extensions.md` |
| Step-by-step build instructions for an agent | `docs/13-agent-playbook.md` |

## Working agreements for AI agents

- **Work phase by phase** (`docs/10-roadmap-and-mvp.md`). Don't start a later phase's features early. Each phase has exit criteria; meet them before moving on.
- **Stay inside the MVP list.** Out of MVP: interactive rebase, stash management UI, submodule management, issues & Actions logs, multi-host support, plugins, Forge World. If a request drifts there, write an ADR/note instead of code.
- **Before adding a dependency**, check it isn't already in the stack and that it doesn't pull I/O into `forge-core`.
- **Write the test with the feature.** Git behaviour is tested on real temp repos, not mocks (see `docs/09`). Destructive actions need a safety test.
- **When a decision is Open** (`docs/11-decisions.md`), implement behind a trait/flag, record the choice you made as a Proposed ADR, and call it out in your summary. Do not silently pick.
- **Never** log or store tokens/secrets; **never** build shell commands from strings (use arg arrays); treat repo paths and branch names as untrusted input.
- **Never** let the TUI leave the terminal in a bad state: raw mode / alt screen must be restored on panic, error and signal.
- **Never** print logs into the TUI or normal CLI output; logs go to a file via `tracing`.

## Definition of done (every change)

- [ ] `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` clean
- [ ] `cargo test --workspace` passes (unit + integration on temp repos)
- [ ] No layer-boundary violations (no `git2`/`reqwest`/`rusqlite` imports in `forge-core`, `forge-tui`, `forge-cli` logic)
- [ ] Errors are user-facing triples: what happened / likely cause / next step (with key binding if in TUI)
- [ ] TUI changes covered by `TestBackend` snapshots at 80x24 and one wider size
- [ ] `--json` output contracts unchanged or version-noted
- [ ] Help content updated in the single shared help source
- [ ] Works at 80x24; respects `NO_COLOR`

## Common commands

```bash
cargo build --workspace
cargo test --workspace
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
cargo run -p forge-cli -- status
cargo run -p forge-cli            # launches TUI
cargo insta review                # approve TUI snapshots
cargo audit && cargo deny check   # dependency audit
```

## Naming note

The product is "Forge". The host-abstraction trait is named **`ForgeHost`** (not `Forge`) to avoid collision with the product name — the Notion spec's service table says "`Forge` trait"; treat that as `ForgeHost`.
