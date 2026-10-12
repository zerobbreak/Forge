# ADR 0003: Pull requests through the GitHub CLI

- **Status:** Proposed
- **Date:** 2026-10-12
- **Decision register ID:** D3 (GitHub auth) and `docs/04-github-integration.md` ("REST for MVP")

## Context
`docs/04-github-integration.md` plans native REST calls through Reqwest or Octocrab. Forge already depends on the GitHub CLI for sign-in, the account switcher and the project list, and its token lives in the CLI's keyring. Screen 05 (Pull requests) is the first screen that reads repository data from GitHub. Adding Reqwest, token handling and pagination just for this would duplicate what `gh` does after the user has already signed in with it.

## Options considered
1. **Native REST** with Reqwest or Octocrab. This is the long-term plan in docs/04, but it needs HTTP, TLS, token storage and cache code that nothing else uses yet.
2. **`gh pr list|view|checkout --json …`** run in the project folder. `gh` finds the repository from the Git remote and uses the active account. Only `serde_json` is added to read its output.

## Decision
Option 2 (`src/github/pulls.rs`), behind plain functions that the screen calls from a worker thread, so a native client can replace them later without changing the screen:
- Commands are built as argument arrays, with `GH_PROMPT_DISABLED`, `NO_COLOR` and no stdin, so `gh` can never hang the TUI.
- Pull request numbers are `u64`, so nothing from the repository reaches a command line as text.
- `gh` failures are turned into a cause and a next step (`explain`): no GitHub remote, not signed in, offline, repository not visible, rate limited.
- Everything on the screen is labelled `from GitHub` with the age of the fetch.
- Missing JSON fields degrade (an empty list, "unknown") instead of failing the screen.

## Evidence
- Parser tests use JSON shaped like real `gh` output (check runs, commit statuses, drafts, merged pull requests, team review requests).
- An ignored test (`cargo test -- --ignored reads_real_pull_requests_through_gh`) lists and reads real pull requests from a public repository and checks the "no GitHub remote" message.

## Consequences
- `gh` must be installed and signed in, which `forge` already requires at start-up.
- Listing pulls the check rollup for every pull request, which is slower than a purpose-built REST query. A page is limited to 50.
- No caching beyond the current visit, and no offline mode yet. Both belong to the native client.
- Creating a pull request, merging, commenting and approving are not built.
