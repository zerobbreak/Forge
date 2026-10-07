---
name: forge-progress
description: Track agent work in Forge, preserve concise local project context, and detect file or behavior conflicts before changes are made.
---

# Forge Agent Progress

Use this workflow for every task in the Forge repository.

## Before editing

1. Read the root `AGENT_PROGRESS.md` if it exists. If it does not exist, create
   it with `# Agent Progress`, `## Active`, and `## Recently completed` sections.
2. Inspect the current worktree status and identify the files or behavior your
   task is likely to change.
3. Compare your scope with active entries and existing uncommitted changes.
   Consider indirect impact too: shared APIs, configuration, schemas, generated
   files, and tests may conflict even when filenames differ.
4. If another active task overlaps, do not silently take over or overwrite its
   work. Coordinate through the available agent/session communication channel;
   narrow the scope or wait if ownership is unclear.
5. Add an active entry before editing. Include an agent/session identifier, a
   concise task, expected files or areas, status, and known dependencies,
   conflicts, or likely impact. Use `unknown` rather than inventing an identity.

## While working

- Update the entry when scope or affected files change, when a meaningful
  milestone is reached, or when a blocker/dependency appears.
- Before touching a newly discovered file, compare it against all active scopes
  and note any newly discovered interaction.
- Keep the log factual and brief. Do not copy source code, secrets, credentials,
  or lengthy conversation history into it.
- The progress file is local and Git-ignored; it coordinates only agents that
  can access this same worktree. Its absence in another worktree is not proof
  that no other agent is working.

## When finishing

- Record the result and any follow-up or compatibility impact in
  `Recently completed`, then remove the task from `Active`.
- If the work is paused or blocked, leave it in `Active` with its status and
  next step instead of marking it complete.
- Keep the completed section to recent, useful context; remove stale entries
  when the file becomes unwieldy.
- In your final response, briefly report relevant overlaps or dependencies
  discovered, especially if they affected the implementation.
