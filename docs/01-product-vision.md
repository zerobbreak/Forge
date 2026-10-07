# 01 — Product Vision, Users & Scope

## Positioning

**One-liner:** Git and GitHub, made visible and learnable, without leaving the terminal.

Forge presents Git state and GitHub collaboration through one consistent interface that is fast enough for experts and self-explanatory enough for learners.

### How Forge differs

| Tool | Strength | Gap Forge targets |
|---|---|---|
| Git CLI | Complete, universal | Hard to discover; state invisible; easy to make destructive mistakes |
| lazygit / gitui | Fast keyboard Git TUIs | Local-Git focused; GitHub (PRs, Actions, reviews) secondary/absent; little teaching |
| GitHub CLI (`gh`) | Excellent GitHub scripting | Command-only; no persistent view of local + remote together |
| IDE Git panels | Editor-integrated | Tied to an editor; unavailable over SSH / minimal environments |

**Differentiators**
1. Git **and** GitHub in one view, with a clear local-vs-remote distinction.
2. Built-in guide and contextual footer.
3. A Forge-owned activity timeline.
4. Event-driven core designed for plugins and a future visualization layer.

**Strategy vs the crowded market:** lead with GitHub integration, learnability and safety — NOT feature parity with lazygit.

## Personas

| Persona | Needs | What Forge gives |
|---|---|---|
| **Learner** (student, bootcamp, junior) | Understand what Git is doing; avoid breaking things | Guide, plain-language errors, visual branches, confirmations |
| **Daily driver** (working dev) | Fast stage→commit→push→PR loop | Hunk staging, keyboard-first TUI, direct CLI commands |
| **Remote/SSH user** | Full workflow on a server/container with no GUI | Single binary; no browser or Python needed |
| **Tinkerer** (later) | Automate and extend | Event stream, Python plugins, themes, keymaps |

## MVP user stories

- Run `forge` in a repo → immediately see branch, ahead/behind, changed files.
- Stage individual hunks, write a commit and push without leaving the TUI.
- Open a pull request from the current branch and see status + checks.
- Press `?` anywhere → see what each key does in the current view.
- Be asked to confirm before discarding changes, and recover them afterwards.
- Run `forge status --json` / `forge pr list --json` → stable machine-readable output.
- Jump between recent and favourite projects from one list.

## Goals (v1)

Terminal-first productivity · one command, two interaction styles · Git + GitHub in one tool · low infrastructure footprint · safe operations · learnability · stable Rust core with a clean path to extensions and a visualization layer.

## Non-goals (v1)

- No Forge cloud backend; no PostgreSQL/Redis/always-on server
- No browser UI
- No Python requirement
- No game layer
- No replacement of Git's object database
- No non-GitHub hosts yet (GitLab, Bitbucket) — but the remote layer MUST sit behind the `ForgeHost` trait

## Product identity

Forge is not a Git command wrapper. It is a terminal-native development environment centred on Git: fast for experts, visual enough to make repository state understandable, safe enough for learners, extensible enough to grow into automation, plugins and an educational visualization layer.

## Success criteria

**Qualitative:** install + launch with one command; useful with no Python/browser UI/hosted service; full edit→stage→commit→push→PR loop inside Forge; local and GitHub state always visually distinct; TUI responsive during network ops; CLI and TUI share the same services; clean path to plugins and Forge World without a core rewrite.

**Measurable (post-release):** time from install to first PR created from Forge · destructive-action confirmations that were *cancelled* (signal the safety UX works) · crash-free session rate · median keypress-to-render latency · guide completion % · GitHub stars/issues/contributors.
