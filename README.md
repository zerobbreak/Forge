# Forge — AI Build Docs

Documentation pack for building **Forge**, a terminal-native Git + GitHub environment (Rust, TUI + CLI, no backend), derived from the Forge specification in Notion.

Start with **`CLAUDE.md`** (put it at the repo root; also usable as `AGENTS.md`), then `docs/`.

```
CLAUDE.md                      ← agent entry point: rules, layout, DoD, doc map
docs/
  01-product-vision.md         ← positioning, personas, scope
  02-architecture.md           ← layers, crates, services, traits
  03-git-engine.md             ← git2 vs CLI fallback
  04-github-integration.md     ← auth, API, caching, offline
  05-data-and-storage.md       ← SQLite schema, config.toml
  06-events-and-concurrency.md ← threads, channels, event catalogue
  07-safety-and-errors.md      ← risk levels, snapshots, errors, logging
  08-cli-and-tui-spec.md       ← commands, keymap, layout, guide
  09-testing-quality-security.md
  10-roadmap-and-mvp.md        ← phases, task checklists, acceptance
  11-decisions.md              ← decision register (D1–D15)
  12-future-extensions.md      ← plugins & Forge World constraints
  13-agent-playbook.md         ← build order, conventions, prompt templates
  adr/0000-template.md
```

## Running Forge

From the repository directory, run Forge without installing it:

```powershell
cargo run
cargo run -- --help
cargo run -- login
cargo run -- account list
cargo run -- account add
cargo run -- account use USERNAME
cargo run -- projects
```

To use `forge` directly in PowerShell, install the binary:

```powershell
cargo install --path .
Get-Command forge
forge
forge login
forge account list
forge account add
forge account use USERNAME
forge projects
```

`forge login` uses the GitHub CLI (`gh auth login`) for device-flow
authentication and credential storage. Install GitHub CLI first if it is not
already available on `PATH`. If PowerShell still cannot find `forge` after
installation, add Cargo's binary directory (`$env:USERPROFILE\.cargo\bin`) to
your user `PATH` and open a new PowerShell window.

`forge projects` lists repositories available to your authenticated GitHub
account, including organization and collaborator repositories.

Running `forge` opens the local Projects browser. It scans `~/projects` for
Git repositories and shows each repository's branch, working-tree changes,
remote, and recent commits. Use the arrow keys (or `j`/`k`) to select a
repository, `/` to filter, `Enter` to open it, and `q` to quit. The Changes
view is still under development.

Manage multiple GitHub.com accounts with `forge account list` to see saved
accounts, `forge account add` to sign in another account, and
`forge account use USERNAME` to switch the active account. Credentials remain
managed by GitHub CLI; projects are fetched using whichever account is active.
