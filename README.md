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
