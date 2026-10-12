# ADR 0001: System `git` CLI for all Git access, with patch-based hunk staging

- **Status:** Proposed
- **Date:** 2026-10-10
- **Decision register ID:** D2, D8

## Context
The documented stack (`docs/03-git-engine.md`, D2) proposes git2 (libgit2) by default with the `git` CLI as a fallback. The code so far has only ever called the `git` binary. Before building the Changes screen (design artboard 01), we had to decide how to stage individual hunks (D8, Open).

## Options considered
1. **git2 only.** One in-process API. But it adds a native C dependency (painful on Windows and OneDrive build folders), and libgit2 lags git on hooks, signing, SSH and credential helpers.
2. **Hybrid (git2 reads, CLI writes).** This was the earlier proposal. It means two code paths and two sets of failure modes.
3. **System `git` CLI only.** It matches existing code, behaves exactly like the user's own git (config, hooks, credentials) and adds no new dependency. The costs are output parsing and process start-up per call.

For hunk staging specifically: (a) git2 `apply` to index, or (b) build a one-hunk patch from `git diff` and pipe it to `git apply --cached`.

## Decision
The user chose **option 3**: every Git operation runs `git -C <repo> …` with an **argument array**, never a shell string (`src/git/repository.rs::run_git`). Patches go in through stdin.

Hunk staging uses **option (b)**:
- Forge keeps each hunk's raw bytes exactly as `git diff` printed them.
- It rebuilds a patch from the file header plus one hunk and runs `git apply --cached` (stage), `--cached --reverse` (unstage) or `--reverse` (discard).
- Diffs force `--no-color --no-ext-diff --src-prefix=a/ --dst-prefix=b/ -U3`, so user config (`diff.noprefix`, external diff tools, `color.diff=always`) cannot break the patch.

## Evidence
`src/git/changes.rs` tests run on real temporary repositories with a two-hunk file. They cover:
- staging and unstaging a single hunk (`git diff --cached` contains only that hunk);
- staging and unstaging whole files in a repository with no commits;
- showing an untracked file's diff via `git diff --no-index -- /dev/null <path>`, which works on Git for Windows 2.48;
- the end-to-end Changes screen flow through the background request path.

## Consequences
- No libgit2 build dependency. Each operation is one or more `git` processes, which is fast enough for interactive use.
- Parsing relies on stable porcelain formats: `status --porcelain=v1 -z`, `diff --numstat -z` and unified diff.
- Renamed, copied, conflicted, untracked and binary files are staged as whole files only.
- Line-level staging (selecting single lines within a hunk) will need patch editing on top of this approach.

## Revisit if
- `git apply` rejects hunks in real-world fixtures (CRLF/`autocrlf`, whitespace rules, filters).
- Process start-up becomes a measurable bottleneck in large repositories.
- A feature needs an API the CLI cannot express cleanly.
