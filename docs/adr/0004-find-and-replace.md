# ADR 0004: Find in files and replace

- **Status:** Proposed
- **Date:** 2026-10-12
- **Decision register ID:** D9 (recovery snapshots), extends ADR 0001 and ADR 0002

## Context
Screen 08 searches every file in a project and can rewrite the matches. Replace is destructive and touches many files at once, so it has to follow `docs/07-safety-and-errors.md`: preview, confirm, recover. The search has to be fast on large repositories, respect `.gitignore`, and never block the TUI.

## Options considered
1. **Walk the files in Rust.** This needs its own ignore-file handling and binary detection, and duplicates what Git already does.
2. **`git grep` finds candidate lines; a Rust regex marks the exact matches and performs the replacement.** Git supplies speed, `.gitignore` and binary detection. Rust supplies the same matching for highlighting, preview and replace.
3. **Replace with `sed` or `perl` through the shell.** This builds commands from strings and behaves differently across platforms. It conflicts with the project rules.

## Decision
Option 2 (`src/git/search.rs`):
- **Search:** `git grep -n -z -I --untracked --exclude-standard`, with `-F` for text and `-E` for regular expressions, `-i` unless case is on, and `-w` for whole words. The query follows `-e`, so a query such as `--force` is text, not a flag. Binary and ignored files are skipped. At most 1000 matches are kept, and lines are cut at 400 characters.
- **Matching:** the `regex` crate builds the matcher from the same options (text is escaped unless regex mode is on). The screen validates the pattern while typing and never asks Git to run an invalid one.
- **Replace** edits only lines the user left selected:
  1. Each target line is checked against the text the search saw. If the file changed since, that line is skipped and counted, never guessed at.
  2. The file is edited in memory, keeping its line endings (`\n`, `\r\n`, or none on the last line).
  3. A recovery snapshot of the file is saved under `refs/forge/snapshots/` first (ADR 0002). Only then is the file written.
  4. Files that are not valid UTF-8, are larger than 4 MiB, or have disappeared are skipped and reported.
  5. Paths must be plain relative paths; anything that leaves the repository is refused.
- **Confirmation:** a red dialog shows the line and file counts, the old and new text of the first four changes, and where the snapshots go. Only `y` confirms; `Enter` does not.
- **Result line:** how many lines and files were changed, how many were left alone, and `Undo: git checkout <ref> -- <path>`.
- **Snapshot names:** found while building this. Snapshot refs were named by the current millisecond, so two snapshots taken in the same millisecond (a multi-file replace) could overwrite one another. The stamp is now strictly increasing within the process, with a test.

## Evidence
Tests on real temporary repositories cover:
- Search: tracked, untracked, ignored and binary files; regex, whole word and case options; a bad pattern; a query that looks like an option.
- Replace: selected lines only, CRLF and no-trailing-newline files, lines that changed since the search, files that cannot be edited, capture groups, `$` in text mode, paths that escape the repository.
- The snapshot of each file holds the original, and the undo command restores it.
- An end-to-end run through the screen's own requests.

## Consequences
- New dependency: `regex` (pure Rust, no I/O). Git's extended regular expressions and Rust's syntax are close but not identical. A line Git reports but the Rust matcher does not match is listed without highlighting and is left unchanged by replace.
- Multi-line patterns are not supported; each line is matched on its own.
- Replace does not stage or commit anything. The changes appear in Changes like any other edit.
- Not built: replace inside a single selected file only, undo from a list of snapshots, and searching a past commit.
