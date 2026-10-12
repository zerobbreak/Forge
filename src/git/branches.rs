//! Branches and worktrees: listing them and the safe operations the Branches screen offers.
//!
//! Branch names come from the repository and from the user, so they are checked
//! here before reaching a command line and always passed as separate arguments.

use super::repository::{GitOptions, git_output, run_git};
use std::{collections::HashSet, path::Path};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BranchKind {
    Local,
    Remote,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Branch {
    /// `main` for a local branch, `origin/main` for a remote one.
    pub name: String,
    pub kind: BranchKind,
    /// Checked out in this working copy.
    pub current: bool,
    pub upstream: Option<String>,
    pub ahead: usize,
    pub behind: usize,
    /// The upstream branch no longer exists on the remote.
    pub gone: bool,
    pub short: String,
    pub hash: String,
    pub subject: String,
    pub time: u64,
    /// Every commit on it is already in `HEAD`, so deleting it loses nothing.
    pub merged: bool,
    /// Commits on it that `HEAD` does not have, when this git can count them.
    pub unique: Option<usize>,
    /// Folder of another worktree that has it checked out.
    pub worktree: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Worktree {
    pub path: String,
    /// `None` when detached or bare.
    pub branch: Option<String>,
    pub short: String,
    pub detached: bool,
    pub bare: bool,
    /// This is the folder Forge was opened in.
    pub current: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BranchList {
    /// Local branches (current first, then newest), then remote branches (newest first).
    pub branches: Vec<Branch>,
    pub worktrees: Vec<Worktree>,
    /// Short id of `HEAD` when no branch is checked out.
    pub detached: Option<String>,
    /// Tracked files with uncommitted changes, which a branch switch may carry or refuse.
    pub dirty_files: usize,
}

impl BranchList {
    pub fn locals(&self) -> impl Iterator<Item = &Branch> {
        self.branches
            .iter()
            .filter(|branch| branch.kind == BranchKind::Local)
    }

    pub fn current(&self) -> Option<&Branch> {
        self.branches.iter().find(|branch| branch.current)
    }

    pub fn local_count(&self) -> usize {
        self.locals().count()
    }
}

const FIELD: char = '\u{1f}';

pub fn read_branches(repo: &Path) -> Result<BranchList, String> {
    let base = format!(
        "%(refname){FIELD}%(HEAD){FIELD}%(upstream:short){FIELD}%(upstream:track){FIELD}%(committerdate:unix){FIELD}%(objectname:short){FIELD}%(objectname){FIELD}%(worktreepath){FIELD}%(contents:subject)"
    );
    let counted = format!("--format={base}{FIELD}%(ahead-behind:HEAD)");
    let plain = format!("--format={base}");
    // `ahead-behind` needs a recent git and a HEAD with commits; fall back without it.
    let (output, counts) = match git_output(
        repo,
        &["for-each-ref", &counted, "refs/heads", "refs/remotes"],
    ) {
        Ok(output) => (output, true),
        Err(_) => (
            git_output(
                repo,
                &["for-each-ref", &plain, "refs/heads", "refs/remotes"],
            )?,
            false,
        ),
    };
    let merged: HashSet<String> = git_output(
        repo,
        &[
            "for-each-ref",
            "--merged=HEAD",
            "--format=%(refname)",
            "refs/heads",
        ],
    )
    .map(|output| {
        String::from_utf8_lossy(&output)
            .lines()
            .map(str::to_owned)
            .collect()
    })
    .unwrap_or_default();

    let mut branches = parse_branches(&String::from_utf8_lossy(&output), counts, &merged);
    branches.sort_by(|a, b| {
        (
            a.kind == BranchKind::Remote,
            !a.current,
            std::cmp::Reverse(a.time),
        )
            .cmp(&(
                b.kind == BranchKind::Remote,
                !b.current,
                std::cmp::Reverse(b.time),
            ))
    });

    let detached = if branches.iter().any(|branch| branch.current) {
        None
    } else {
        git_output(repo, &["rev-parse", "--short", "HEAD"])
            .ok()
            .map(|out| String::from_utf8_lossy(&out).trim().to_owned())
            .filter(|short| !short.is_empty())
    };
    let worktrees = git_output(repo, &["worktree", "list", "--porcelain"])
        .map(|out| parse_worktrees(&String::from_utf8_lossy(&out), repo))
        .unwrap_or_default();
    let dirty_files = git_output(repo, &["status", "--porcelain", "--untracked-files=no"])?
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .count();
    Ok(BranchList {
        branches,
        worktrees,
        detached,
        dirty_files,
    })
}

/// Parses `for-each-ref` output printed with [`read_branches`]'s format.
pub fn parse_branches(output: &str, counts: bool, merged: &HashSet<String>) -> Vec<Branch> {
    output
        .lines()
        .filter_map(|line| {
            let mut fields = line.splitn(10, FIELD);
            let refname = fields.next()?;
            let head = fields.next()?;
            let upstream = fields.next()?;
            let track = fields.next()?;
            let time = fields.next()?.parse().unwrap_or(0);
            let short = fields.next()?.to_owned();
            let hash = fields.next()?.to_owned();
            let worktree = fields.next()?;
            let subject = fields.next().unwrap_or_default();
            let ahead_behind = if counts {
                fields.next().unwrap_or_default()
            } else {
                ""
            };

            let (kind, name) = if let Some(name) = refname.strip_prefix("refs/heads/") {
                (BranchKind::Local, name)
            } else {
                // `origin/HEAD` is a pointer to another remote branch, not a branch.
                let name = refname
                    .strip_prefix("refs/remotes/")
                    .filter(|name| !name.ends_with("/HEAD"))?;
                (BranchKind::Remote, name)
            };
            let current = head == "*";
            let (ahead, behind, gone) = parse_track(track);
            let unique = ahead_behind
                .split_whitespace()
                .next()
                .and_then(|ahead| ahead.parse().ok());
            let is_merged = kind == BranchKind::Local && merged.contains(refname);
            Some(Branch {
                name: name.to_owned(),
                kind,
                current,
                upstream: (!upstream.is_empty()).then(|| upstream.to_owned()),
                ahead,
                behind,
                gone,
                short,
                hash,
                subject: subject.to_owned(),
                time,
                merged: is_merged,
                unique: if is_merged { Some(0) } else { unique },
                worktree: (kind == BranchKind::Local && !current && !worktree.is_empty())
                    .then(|| worktree.to_owned()),
            })
        })
        .collect()
}

/// Parses `%(upstream:track)`: `[ahead 1, behind 2]`, `[gone]` or empty.
fn parse_track(track: &str) -> (usize, usize, bool) {
    let inner = track.trim().trim_start_matches('[').trim_end_matches(']');
    if inner == "gone" {
        return (0, 0, true);
    }
    let (mut ahead, mut behind) = (0, 0);
    for part in inner.split(", ") {
        if let Some(count) = part.strip_prefix("ahead ") {
            ahead = count.parse().unwrap_or(0);
        } else if let Some(count) = part.strip_prefix("behind ") {
            behind = count.parse().unwrap_or(0);
        }
    }
    (ahead, behind, false)
}

/// Parses `git worktree list --porcelain`; `repo` marks which entry is the current folder.
pub fn parse_worktrees(output: &str, repo: &Path) -> Vec<Worktree> {
    let here = same_folder_key(&repo.display().to_string());
    output
        .split("\n\n")
        .filter_map(|block| {
            let mut path = None;
            let mut worktree = Worktree {
                path: String::new(),
                branch: None,
                short: String::new(),
                detached: false,
                bare: false,
                current: false,
            };
            for line in block.lines() {
                if let Some(value) = line.strip_prefix("worktree ") {
                    path = Some(value.to_owned());
                } else if let Some(value) = line.strip_prefix("HEAD ") {
                    worktree.short = value.chars().take(7).collect();
                } else if let Some(value) = line.strip_prefix("branch ") {
                    worktree.branch = Some(
                        value
                            .strip_prefix("refs/heads/")
                            .unwrap_or(value)
                            .to_owned(),
                    );
                } else if line == "detached" {
                    worktree.detached = true;
                } else if line == "bare" {
                    worktree.bare = true;
                }
            }
            let path = path?;
            worktree.current = same_folder_key(&path) == here;
            worktree.path = path;
            Some(worktree)
        })
        .collect()
}

/// A path in a form where git's `C:/x/y` and Windows' `C:\x\y` compare equal.
fn same_folder_key(path: &str) -> String {
    let path = path.strip_prefix(r"\\?\").unwrap_or(path);
    path.replace('\\', "/").trim_end_matches('/').to_lowercase()
}

/// Checks a branch name without running git, so the screen can answer while typing.
pub fn check_branch_name(name: &str) -> Result<(), String> {
    let invalid = |why: &str| Err(format!("“{name}” is not a valid branch name: {why}."));
    if name.is_empty() {
        return Err("Type a branch name first.".to_owned());
    }
    if name.starts_with('-') {
        return invalid("it cannot start with a dash");
    }
    if name == "@" || name.contains("@{") {
        return invalid("“@” has a special meaning in Git");
    }
    if name.starts_with('/') || name.ends_with('/') || name.contains("//") {
        return invalid("slashes must separate names");
    }
    if name.ends_with('.') || name.ends_with(".lock") || name.contains("..") {
        return invalid("it cannot end in “.” or “.lock”, or contain “..”");
    }
    if let Some(bad) = name
        .chars()
        .find(|c| c.is_whitespace() || c.is_control() || "~^:?*[\\".contains(*c))
    {
        return invalid(&format!("it cannot contain “{}”", bad.escape_default()));
    }
    if name.split('/').any(|part| part.starts_with('.')) {
        return invalid("no part may start with “.”");
    }
    Ok(())
}

fn run(repo: &Path, arguments: &[&str]) -> Result<Vec<u8>, String> {
    run_git(repo, arguments, GitOptions::default()).map_err(|detail| brief(&detail))
}

/// Git's multi-line complaints, cut down to what fits in a status line.
fn brief(detail: &str) -> String {
    let lines: Vec<&str> = detail
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("hint:"))
        .collect();
    let mut text = lines.join(" ");
    if text.chars().count() > 200 {
        text = text.chars().take(199).collect::<String>() + "…";
    }
    text
}

fn local_exists(repo: &Path, name: &str) -> Result<bool, String> {
    let reference = format!("refs/heads/{name}");
    let output = run_git(
        repo,
        &["rev-parse", "--verify", "--quiet", &reference],
        GitOptions {
            allowed_codes: &[1],
            ..GitOptions::default()
        },
    )?;
    Ok(!String::from_utf8_lossy(&output).trim().is_empty())
}

/// Where a switch goes: an existing local branch, or a new local one tracking a remote branch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SwitchTarget {
    pub name: String,
    /// `origin/feat` when `name` has to be created from it.
    pub track: Option<String>,
}

/// Switches branch. With `stash`, uncommitted tracked changes are stashed first and
/// put back with `git stash pop` if the switch fails.
pub fn switch_branch(repo: &Path, target: &SwitchTarget, stash: bool) -> Result<String, String> {
    check_branch_name(&target.name)?;
    let mut stashed = false;
    if stash {
        let message = format!("forge: before switching to {}", target.name);
        let output = run(repo, &["stash", "push", "-m", &message])?;
        stashed = !String::from_utf8_lossy(&output).contains("No local changes");
    }
    let switched = match &target.track {
        Some(remote) => {
            check_branch_name(remote)?;
            run(repo, &["switch", "--track", remote])
        }
        None => run(repo, &["switch", &target.name]),
    };
    if let Err(cause) = switched {
        if stashed {
            let restored = run(repo, &["stash", "pop"]).is_ok();
            return Err(if restored {
                format!("{cause} Your changes were put back.")
            } else {
                format!("{cause} Your changes are still stashed: run git stash pop.")
            });
        }
        return Err(cause);
    }
    Ok(if stashed {
        format!(
            "Switched to {}. Your changes are stashed; bring them back with: git stash pop",
            target.name
        )
    } else {
        format!("Switched to {}.", target.name)
    })
}

/// Creates `name` at `start` (a full ref such as `refs/heads/main`) and switches to it.
pub fn create_branch(repo: &Path, name: &str, start: &str) -> Result<String, String> {
    check_branch_name(name)?;
    if local_exists(repo, name)? {
        return Err(format!("A branch named {name} already exists."));
    }
    run(repo, &["switch", "-c", name, start])?;
    Ok(format!("Created {name} and switched to it."))
}

pub fn rename_branch(repo: &Path, old: &str, new: &str) -> Result<String, String> {
    check_branch_name(old)?;
    check_branch_name(new)?;
    if old == new {
        return Err("The new name is the same as the old one.".to_owned());
    }
    if local_exists(repo, new)? {
        return Err(format!("A branch named {new} already exists."));
    }
    run(repo, &["branch", "-m", old, new])?;
    Ok(format!("Renamed {old} to {new}."))
}

/// Deletes a local branch. `force` is needed when it has commits `HEAD` lacks; the
/// result names the command that brings it back.
pub fn delete_branch(repo: &Path, name: &str, force: bool) -> Result<String, String> {
    check_branch_name(name)?;
    let reference = format!("refs/heads/{name}");
    let hash = String::from_utf8_lossy(&run(repo, &["rev-parse", "--verify", &reference])?)
        .trim()
        .to_owned();
    run(repo, &["branch", if force { "-D" } else { "-d" }, name])?;
    let short: String = hash.chars().take(7).collect();
    Ok(format!(
        "Deleted {name} (was {short}). Undo with: git branch {name} {hash}"
    ))
}

#[cfg(test)]
mod tests {
    use super::{
        BranchKind, SwitchTarget, check_branch_name, create_branch, delete_branch, parse_branches,
        parse_track, parse_worktrees, read_branches, rename_branch, switch_branch,
    };
    use crate::test_support::{TestDirectory, git_text, run_git};
    use std::{
        collections::HashSet,
        fs,
        path::{Path, PathBuf},
    };

    fn repository(directory: &TestDirectory) -> PathBuf {
        let repo = directory.path().join("repo");
        fs::create_dir_all(&repo).unwrap();
        run_git(&repo, &["init", "-b", "main"]);
        run_git(&repo, &["config", "user.name", "Forge Test"]);
        run_git(&repo, &["config", "user.email", "forge@example.test"]);
        run_git(&repo, &["config", "commit.gpgsign", "false"]);
        repo
    }

    fn commit(repo: &Path, file: &str, text: &str, message: &str) {
        fs::write(repo.join(file), text).unwrap();
        run_git(repo, &["add", file]);
        run_git(repo, &["commit", "-m", message]);
    }

    fn names(list: &super::BranchList) -> Vec<&str> {
        list.branches.iter().map(|b| b.name.as_str()).collect()
    }

    #[test]
    fn parses_upstream_tracking_text() {
        assert_eq!(parse_track("[ahead 3, behind 12]"), (3, 12, false));
        assert_eq!(parse_track("[ahead 2]"), (2, 0, false));
        assert_eq!(parse_track("[behind 1]"), (0, 1, false));
        assert_eq!(parse_track("[gone]"), (0, 0, true));
        assert_eq!(parse_track(""), (0, 0, false));
    }

    #[test]
    fn parses_branch_records_and_skips_remote_head_pointers() {
        let merged: HashSet<String> = ["refs/heads/main".to_owned()].into();
        let record = |fields: [&str; 9]| fields.join("\u{1f}");
        let output = [
            record([
                "refs/heads/main",
                "*",
                "origin/main",
                "[ahead 1]",
                "100",
                "aaaaaaa",
                "aaaaaaa0",
                "C:/repo",
                "feat: one",
            ]) + "\u{1f}0 0",
            record([
                "refs/heads/feat/x",
                " ",
                "",
                "",
                "90",
                "bbbbbbb",
                "bbbbbbb0",
                "C:/other",
                "wip",
            ]) + "\u{1f}2 0",
            record([
                "refs/remotes/origin/HEAD",
                " ",
                "",
                "",
                "100",
                "aaaaaaa",
                "aaaaaaa0",
                "",
                "feat: one",
            ]) + "\u{1f}0 0",
            record([
                "refs/remotes/origin/main",
                " ",
                "",
                "",
                "100",
                "aaaaaaa",
                "aaaaaaa0",
                "",
                "feat: one",
            ]) + "\u{1f}0 0",
        ]
        .join("\n");
        let branches = parse_branches(&output, true, &merged);
        assert_eq!(branches.len(), 3);
        assert!(branches[0].current && branches[0].merged);
        assert_eq!((branches[0].ahead, branches[0].behind), (1, 0));
        assert_eq!(
            branches[0].worktree, None,
            "the current branch is not 'elsewhere'"
        );
        assert_eq!(branches[1].name, "feat/x");
        assert_eq!(branches[1].unique, Some(2));
        assert!(!branches[1].merged);
        assert_eq!(branches[1].worktree.as_deref(), Some("C:/other"));
        assert_eq!(branches[2].kind, BranchKind::Remote);
        assert_eq!(branches[2].name, "origin/main");
    }

    #[test]
    fn parses_worktrees_and_finds_the_current_folder() {
        let output = "worktree C:/work/repo\nHEAD aaaaaaaaaaaa\nbranch refs/heads/main\n\nworktree C:/work/repo-fix\nHEAD bbbbbbbbbbbb\nbranch refs/heads/fix\n\nworktree C:/work/old\nHEAD cccccccccccc\ndetached\n";
        let trees = parse_worktrees(output, Path::new(r"C:\work\Repo"));
        assert_eq!(trees.len(), 3);
        assert!(trees[0].current && !trees[1].current);
        assert_eq!(trees[1].branch.as_deref(), Some("fix"));
        assert_eq!(trees[1].short, "bbbbbbb");
        assert!(trees[2].detached && trees[2].branch.is_none());
    }

    #[test]
    fn validates_branch_names_before_running_git() {
        for good in ["main", "feat/redis-cache", "fix_1.2", "release/v1.0"] {
            assert!(check_branch_name(good).is_ok(), "{good}");
        }
        for bad in [
            "",
            "-x",
            "--force",
            "a b",
            "a..b",
            "a~1",
            "a^",
            "a:b",
            "a?",
            "a*",
            "a[",
            "a\\b",
            "x.lock",
            "x.",
            "/x",
            "x/",
            "a//b",
            "@",
            "a@{u}",
            ".hidden",
            "a/.b",
            "tab\there",
        ] {
            assert!(check_branch_name(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn lists_branches_with_merge_state_and_a_current_marker() {
        let directory = TestDirectory::new("branches-list");
        let repo = repository(&directory);
        commit(&repo, "a.txt", "1\n", "feat: first");
        run_git(&repo, &["branch", "merged-already"]);
        run_git(&repo, &["switch", "-c", "feat/ahead"]);
        commit(&repo, "b.txt", "2\n", "feat: ahead");
        run_git(&repo, &["switch", "main"]);

        let list = read_branches(&repo).expect("branches read");
        assert_eq!(names(&list)[0], "main", "the current branch is first");
        assert!(list.current().unwrap().current);
        assert_eq!(list.local_count(), 3);
        let merged = list
            .branches
            .iter()
            .find(|b| b.name == "merged-already")
            .unwrap();
        assert!(merged.merged);
        let ahead = list
            .branches
            .iter()
            .find(|b| b.name == "feat/ahead")
            .unwrap();
        assert!(!ahead.merged);
        assert_eq!(ahead.unique, Some(1));
        assert_eq!(ahead.subject, "feat: ahead");
        assert_eq!(ahead.short.len(), 7);
        assert!(list.detached.is_none());
        assert_eq!(list.dirty_files, 0);
        assert_eq!(list.worktrees.len(), 1);
        assert!(list.worktrees[0].current);
    }

    #[test]
    fn lists_remote_branches_and_upstream_counts() {
        let directory = TestDirectory::new("branches-remote");
        let remote = directory.path().join("remote.git");
        fs::create_dir_all(&remote).unwrap();
        run_git(&remote, &["init", "--bare", "-b", "main"]);
        let repo = repository(&directory);
        commit(&repo, "a.txt", "1\n", "feat: first");
        run_git(
            &repo,
            &["remote", "add", "origin", remote.to_str().unwrap()],
        );
        run_git(&repo, &["push", "-u", "origin", "main"]);
        commit(&repo, "a.txt", "2\n", "feat: second");

        let list = read_branches(&repo).unwrap();
        let main = list.current().unwrap();
        assert_eq!(main.upstream.as_deref(), Some("origin/main"));
        assert_eq!((main.ahead, main.behind), (1, 0));
        let remotes: Vec<_> = list
            .branches
            .iter()
            .filter(|b| b.kind == BranchKind::Remote)
            .collect();
        assert_eq!(remotes.len(), 1);
        assert_eq!(remotes[0].name, "origin/main");
    }

    #[test]
    fn a_new_repository_without_commits_lists_nothing_and_does_not_fail() {
        let directory = TestDirectory::new("branches-unborn");
        let repo = repository(&directory);
        let list = read_branches(&repo).expect("unborn is not an error");
        assert!(list.branches.is_empty());
    }

    #[test]
    fn creates_switches_renames_and_deletes() {
        let directory = TestDirectory::new("branches-ops");
        let repo = repository(&directory);
        commit(&repo, "a.txt", "1\n", "feat: first");

        create_branch(&repo, "feat/new", "refs/heads/main").expect("creates");
        assert_eq!(
            git_text(&repo, &["branch", "--show-current"]).trim(),
            "feat/new"
        );
        assert!(
            create_branch(&repo, "feat/new", "refs/heads/main")
                .unwrap_err()
                .contains("already exists")
        );
        assert!(create_branch(&repo, "bad name", "refs/heads/main").is_err());

        let target = SwitchTarget {
            name: "main".to_owned(),
            track: None,
        };
        assert_eq!(
            switch_branch(&repo, &target, false).unwrap(),
            "Switched to main."
        );

        rename_branch(&repo, "feat/new", "feat/renamed").expect("renames");
        let list = read_branches(&repo).unwrap();
        assert!(names(&list).contains(&"feat/renamed"));
        assert!(!names(&list).contains(&"feat/new"));
        assert!(rename_branch(&repo, "feat/renamed", "main").is_err());

        let message = delete_branch(&repo, "feat/renamed", false).expect("merged branch deletes");
        assert!(message.contains("Undo with: git branch feat/renamed "));
        assert!(!names(&read_branches(&repo).unwrap()).contains(&"feat/renamed"));
    }

    #[test]
    fn refuses_to_delete_unmerged_work_unless_forced_and_says_how_to_undo() {
        let directory = TestDirectory::new("branches-delete");
        let repo = repository(&directory);
        commit(&repo, "a.txt", "1\n", "feat: first");
        run_git(&repo, &["switch", "-c", "wip"]);
        commit(&repo, "b.txt", "2\n", "wip: unfinished");
        let tip = git_text(&repo, &["rev-parse", "HEAD"]).trim().to_owned();
        run_git(&repo, &["switch", "main"]);

        assert!(delete_branch(&repo, "wip", false).is_err());
        assert!(names(&read_branches(&repo).unwrap()).contains(&"wip"));

        let message = delete_branch(&repo, "wip", true).expect("forced delete");
        assert!(message.ends_with(&format!("Undo with: git branch wip {tip}")));
        // The promised command really brings the work back.
        run_git(&repo, &["branch", "wip", &tip]);
        assert!(names(&read_branches(&repo).unwrap()).contains(&"wip"));

        assert!(
            delete_branch(&repo, "main", true).is_err(),
            "never the current branch"
        );
    }

    #[test]
    fn switching_with_uncommitted_changes_can_stash_and_restores_nothing_on_success() {
        let directory = TestDirectory::new("branches-stash");
        let repo = repository(&directory);
        commit(&repo, "a.txt", "1\n", "feat: first");
        run_git(&repo, &["switch", "-c", "other"]);
        commit(&repo, "a.txt", "other\n", "feat: other change");
        run_git(&repo, &["switch", "main"]);
        fs::write(repo.join("a.txt"), "local edit\n").unwrap();
        assert_eq!(read_branches(&repo).unwrap().dirty_files, 1);

        let target = SwitchTarget {
            name: "other".to_owned(),
            track: None,
        };
        // Carrying the edit over conflicts, so git refuses and nothing moves.
        let refused = switch_branch(&repo, &target, false).unwrap_err();
        assert!(refused.contains("overwritten"), "{refused}");
        assert_eq!(
            git_text(&repo, &["branch", "--show-current"]).trim(),
            "main"
        );
        assert_eq!(
            fs::read_to_string(repo.join("a.txt"))
                .unwrap()
                .replace("\r\n", "\n"),
            "local edit\n"
        );

        let message = switch_branch(&repo, &target, true).expect("stash and switch");
        assert!(message.contains("git stash pop"));
        assert_eq!(
            git_text(&repo, &["branch", "--show-current"]).trim(),
            "other"
        );
        assert_eq!(read_branches(&repo).unwrap().dirty_files, 0);
        assert!(git_text(&repo, &["stash", "list"]).contains("forge: before switching to other"));
    }

    #[test]
    fn a_failed_switch_after_stashing_puts_the_changes_back() {
        let directory = TestDirectory::new("branches-restore");
        let repo = repository(&directory);
        commit(&repo, "a.txt", "1\n", "feat: first");
        fs::write(repo.join("a.txt"), "local edit\n").unwrap();
        let target = SwitchTarget {
            name: "does-not-exist".to_owned(),
            track: None,
        };
        let error = switch_branch(&repo, &target, true).unwrap_err();
        assert!(error.contains("Your changes were put back."), "{error}");
        assert_eq!(
            fs::read_to_string(repo.join("a.txt"))
                .unwrap()
                .replace("\r\n", "\n"),
            "local edit\n"
        );
        assert!(git_text(&repo, &["stash", "list"]).trim().is_empty());
    }

    #[test]
    fn switching_to_a_remote_branch_creates_a_tracking_branch() {
        let directory = TestDirectory::new("branches-track");
        let remote = directory.path().join("remote.git");
        fs::create_dir_all(&remote).unwrap();
        run_git(&remote, &["init", "--bare", "-b", "main"]);
        let repo = repository(&directory);
        commit(&repo, "a.txt", "1\n", "feat: first");
        run_git(
            &repo,
            &["remote", "add", "origin", remote.to_str().unwrap()],
        );
        run_git(&repo, &["push", "-u", "origin", "main"]);
        run_git(&repo, &["switch", "-c", "feat/remote-only"]);
        run_git(&repo, &["push", "-u", "origin", "feat/remote-only"]);
        run_git(&repo, &["switch", "main"]);
        run_git(&repo, &["branch", "-D", "feat/remote-only"]);

        let target = SwitchTarget {
            name: "feat/remote-only".to_owned(),
            track: Some("origin/feat/remote-only".to_owned()),
        };
        switch_branch(&repo, &target, false).expect("creates a tracking branch");
        let list = read_branches(&repo).unwrap();
        let current = list.current().unwrap();
        assert_eq!(current.name, "feat/remote-only");
        assert_eq!(current.upstream.as_deref(), Some("origin/feat/remote-only"));
    }

    #[test]
    fn lists_a_linked_worktree_and_the_branch_it_holds() {
        let directory = TestDirectory::new("branches-worktree");
        let repo = repository(&directory);
        commit(&repo, "a.txt", "1\n", "feat: first");
        let linked = directory.path().join("linked");
        run_git(
            &repo,
            &[
                "worktree",
                "add",
                "-b",
                "fix/linked",
                linked.to_str().unwrap(),
            ],
        );
        let list = read_branches(&repo).unwrap();
        assert_eq!(list.worktrees.len(), 2);
        assert!(list.worktrees.iter().any(|w| w.current));
        let held = list
            .branches
            .iter()
            .find(|b| b.name == "fix/linked")
            .unwrap();
        assert!(held.worktree.is_some(), "checked out elsewhere");
        // Seen from the linked folder, the main branch is the one held elsewhere.
        let from_linked = read_branches(&linked).unwrap();
        assert_eq!(from_linked.current().unwrap().name, "fix/linked");
        assert!(
            from_linked
                .branches
                .iter()
                .find(|b| b.name == "main")
                .unwrap()
                .worktree
                .is_some()
        );
    }
}
