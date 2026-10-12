# ADR 0002: Recovery snapshots as commits under `refs/forge/snapshots/`

- **Status:** Proposed
- **Date:** 2026-10-10
- **Decision register ID:** D9

## Context
`docs/07-safety-and-errors.md` requires an automatic recovery snapshot before destructive local actions "where possible". Screen 01 (Changes) is the first destructive action: discarding a hunk or a file, including deleting untracked files. `git stash create` does not capture untracked files. A separate object store would duplicate what Git already does well.

## Options considered
1. `git stash create` and store the result under a Forge ref. This misses untracked files and captures the whole tree.
2. Copy the files into a Forge folder under `.git/forge/`. This is simple, but it sits outside Git's object model and needs its own pruning and restore code.
3. **A per-path commit built with a temporary index.** Forge sets `GIT_INDEX_FILE` to `.git/forge-snapshot.index`, then runs `read-tree HEAD`, `add -A -f -- <path>`, `write-tree` and `commit-tree -p HEAD`, followed by `update-ref refs/forge/snapshots/<unix-ms>`.

## Decision
Option 3 (`src/git/changes.rs::snapshot`):
- The real index and working tree are never touched.
- The commit uses a fixed `Forge <forge@localhost>` identity, so it works without `user.name`/`user.email` set.
- It also works in repositories with no commits yet (it creates a parent-less commit).
- If the snapshot fails, the discard is aborted.
- The result line shows how to undo: `git checkout <ref> -- <path>`.

## Evidence
Tests on real temporary repositories:
- After discarding a hunk, checking the snapshot out restores the exact edited file, and the real index stays empty.
- An untracked file is deleted only after a snapshot exists, and checking that snapshot out brings the file back.
- Discarding a tracked file's unstaged changes keeps its staged changes.

## Consequences
- Snapshots are ordinary Git objects. They survive `gc` because a ref points at them, and `git log refs/forge/snapshots/…` can inspect them.
- They accumulate. Pruning by age/count (D9) and an in-TUI Undo list are still to build.
- Very large or binary files are copied into the object database on every discard.

## Revisit if
Snapshot cost on large or binary repositories becomes noticeable, or the Undo list needs metadata that refs cannot carry.
