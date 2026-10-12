//! Working-tree changes: status, per-file diffs, staging by file or hunk, and
//! discarding with a recovery snapshot first.
//!
//! Hunk staging builds a one-hunk patch from `git diff` output and feeds it to
//! `git apply --cached` (ADR 0001). Discards record the file's current content
//! under `refs/forge/snapshots/` before touching it (ADR 0002).

use super::repository::{BranchTracking, GitOptions, git_output, parse_branch_line, run_git};
use std::{
    collections::HashMap,
    fs,
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

/// Untracked files larger than this are not read to count their lines.
const MAX_COUNTED_BYTES: u64 = 1024 * 1024;
/// Namespace for recovery snapshots taken before destructive actions.
pub const SNAPSHOT_NAMESPACE: &str = "refs/forge/snapshots";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileState {
    Modified,
    Added,
    Deleted,
    Renamed,
    Copied,
    TypeChanged,
    Untracked,
    Conflicted,
}

impl FileState {
    fn from_code(code: u8) -> Option<Self> {
        match code {
            b'M' => Some(Self::Modified),
            b'A' => Some(Self::Added),
            b'D' => Some(Self::Deleted),
            b'R' => Some(Self::Renamed),
            b'C' => Some(Self::Copied),
            b'T' => Some(Self::TypeChanged),
            _ => None,
        }
    }

    /// The one-character status column: `M`, `+`, `D`, `R`, `C`, `T`, `?`, `U`.
    pub fn symbol(self) -> char {
        match self {
            Self::Modified => 'M',
            Self::Added => '+',
            Self::Deleted => 'D',
            Self::Renamed => 'R',
            Self::Copied => 'C',
            Self::TypeChanged => 'T',
            Self::Untracked => '?',
            Self::Conflicted => 'U',
        }
    }
}

/// One file in the staged or unstaged list. The same path can appear in both.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileChange {
    /// Repository-relative path with `/` separators.
    pub path: String,
    /// The path before a rename or copy.
    pub original: Option<String>,
    pub state: FileState,
    pub staged: bool,
    /// Line counts; `None` for binary files or when they could not be counted.
    pub added: Option<usize>,
    pub removed: Option<usize>,
}

impl FileChange {
    /// Staging a part of the file is possible (text change of a tracked path).
    pub fn supports_hunks(&self) -> bool {
        !matches!(
            self.state,
            FileState::Untracked | FileState::Conflicted | FileState::Renamed | FileState::Copied
        )
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WorkingTree {
    pub tracking: BranchTracking,
    pub staged: Vec<FileChange>,
    pub unstaged: Vec<FileChange>,
    pub branches: usize,
    /// When `git fetch` last wrote `FETCH_HEAD`, as Unix seconds.
    pub last_fetch: Option<u64>,
}

impl WorkingTree {
    /// Distinct paths with any change, staged or not.
    pub fn changed_files(&self) -> usize {
        let mut paths: Vec<&str> = self
            .staged
            .iter()
            .chain(&self.unstaged)
            .map(|change| change.path.as_str())
            .collect();
        paths.sort_unstable();
        paths.dedup();
        paths.len()
    }
}

/// Reads branch tracking, staged and unstaged files with line counts.
pub fn read_working_tree(repo: &Path) -> Result<WorkingTree, String> {
    let status = git_output(
        repo,
        &[
            "status",
            "--porcelain=v1",
            "-z",
            "--branch",
            "--untracked-files=all",
        ],
    )?;
    let (tracking, mut staged, mut unstaged) = parse_status(&status);

    let unstaged_counts = parse_numstat(&git_output(repo, &["diff", "--numstat", "-z"])?);
    let staged_counts = parse_numstat(&git_output(
        repo,
        &["diff", "--cached", "--numstat", "-z", "-M"],
    )?);
    for (changes, counts) in [
        (&mut staged, &staged_counts),
        (&mut unstaged, &unstaged_counts),
    ] {
        for change in changes.iter_mut() {
            if change.state == FileState::Untracked {
                change.added = count_lines(&repo.join(&change.path));
                change.removed = Some(0);
            } else if let Some((added, removed)) = counts.get(&change.path) {
                change.added = *added;
                change.removed = *removed;
            }
        }
    }

    let branches = git_output(repo, &["for-each-ref", "--format=%(refname)", "refs/heads"])?
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .count();
    let last_fetch = git_path(repo, "FETCH_HEAD")
        .ok()
        .and_then(|path| fs::metadata(path).ok())
        .and_then(|metadata| metadata.modified().ok())
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|elapsed| elapsed.as_secs());

    Ok(WorkingTree {
        tracking,
        staged,
        unstaged,
        branches,
        last_fetch,
    })
}

/// Parses `git status --porcelain=v1 -z --branch`.
pub fn parse_status(output: &[u8]) -> (BranchTracking, Vec<FileChange>, Vec<FileChange>) {
    let mut tracking = BranchTracking::default();
    let mut staged = Vec::new();
    let mut unstaged = Vec::new();
    let mut records = output
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty());
    while let Some(record) = records.next() {
        if let Some(header) = record.strip_prefix(b"## ") {
            tracking = parse_branch_line(&String::from_utf8_lossy(header));
            continue;
        }
        if record.len() < 4 {
            continue;
        }
        let (x, y) = (record[0], record[1]);
        let path = String::from_utf8_lossy(&record[3..]).into_owned();
        let change = |state: FileState, staged: bool, original: Option<String>| FileChange {
            path: path.clone(),
            original,
            state,
            staged,
            added: None,
            removed: None,
        };
        match (x, y) {
            (b'!', b'!') => {}
            (b'?', b'?') => unstaged.push(change(FileState::Untracked, false, None)),
            (b'U', _) | (_, b'U') | (b'A', b'A') | (b'D', b'D') => {
                unstaged.push(change(FileState::Conflicted, false, None));
            }
            _ => {
                let original = if matches!(x, b'R' | b'C') {
                    records
                        .next()
                        .map(|from| String::from_utf8_lossy(from).into_owned())
                } else {
                    None
                };
                if let Some(state) = FileState::from_code(x) {
                    staged.push(change(state, true, original));
                }
                if let Some(state) = FileState::from_code(y) {
                    unstaged.push(change(state, false, None));
                }
            }
        }
    }
    (tracking, staged, unstaged)
}

type LineCounts = HashMap<String, (Option<usize>, Option<usize>)>;

/// Parses `git diff --numstat -z`; binary files have `None` counts.
pub fn parse_numstat(output: &[u8]) -> LineCounts {
    let mut counts = HashMap::new();
    let mut records = output.split(|byte| *byte == 0);
    while let Some(record) = records.next() {
        let text = String::from_utf8_lossy(record);
        let mut fields = text.splitn(3, '\t');
        let (Some(added), Some(removed), Some(path)) =
            (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        // Renames print an empty path, then the old and new paths as two records.
        let path = if path.is_empty() {
            let _from = records.next();
            match records.next() {
                Some(to) => String::from_utf8_lossy(to).into_owned(),
                None => continue,
            }
        } else {
            path.to_owned()
        };
        counts.insert(path, (added.parse().ok(), removed.parse().ok()));
    }
    counts
}

fn count_lines(path: &Path) -> Option<usize> {
    let metadata = fs::metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_COUNTED_BYTES {
        return None;
    }
    let bytes = fs::read(path).ok()?;
    if bytes.contains(&0) {
        return None;
    }
    let newlines = bytes.iter().filter(|byte| **byte == b'\n').count();
    Some(newlines + usize::from(bytes.last().is_some_and(|byte| *byte != b'\n')))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    Context,
    Added,
    Removed,
    /// `\ No newline at end of file`.
    Marker,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: LineKind,
    pub text: String,
    pub old: Option<u32>,
    pub new: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hunk {
    /// The `@@ -a,b +c,d @@ context` line.
    pub header: String,
    pub lines: Vec<DiffLine>,
    /// The hunk exactly as git printed it, header included, for `git apply`.
    raw: Vec<u8>,
}

impl Hunk {
    pub fn added(&self) -> usize {
        self.count(LineKind::Added)
    }

    pub fn removed(&self) -> usize {
        self.count(LineKind::Removed)
    }

    fn count(&self, kind: LineKind) -> usize {
        self.lines.iter().filter(|line| line.kind == kind).count()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileDiff {
    /// `diff --git`, `index`, `---` and `+++` lines, exactly as printed.
    header: Vec<u8>,
    pub hunks: Vec<Hunk>,
    pub binary: bool,
}

impl FileDiff {
    /// A patch containing only hunk `index`, ready for `git apply`.
    pub fn hunk_patch(&self, index: usize) -> Option<Vec<u8>> {
        let hunk = self.hunks.get(index)?;
        let mut patch = self.header.clone();
        patch.extend_from_slice(&hunk.raw);
        Some(patch)
    }
}

/// Parses the unified diff of a single file.
pub fn parse_diff(output: &[u8]) -> FileDiff {
    let mut diff = FileDiff::default();
    let mut old = 0;
    let mut new = 0;
    for raw_line in output.split_inclusive(|byte| *byte == b'\n') {
        let line = raw_line.strip_suffix(b"\n").unwrap_or(raw_line);
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if line.starts_with(b"@@") {
            let header = String::from_utf8_lossy(line).into_owned();
            (old, new) = hunk_starts(&header);
            diff.hunks.push(Hunk {
                header,
                lines: Vec::new(),
                raw: raw_line.to_vec(),
            });
            continue;
        }
        let Some(hunk) = diff.hunks.last_mut() else {
            if line.starts_with(b"Binary files ") || line.starts_with(b"GIT binary patch") {
                diff.binary = true;
            }
            diff.header.extend_from_slice(raw_line);
            continue;
        };
        hunk.raw.extend_from_slice(raw_line);
        let (kind, text) = match line.first() {
            Some(b'+') => (LineKind::Added, &line[1..]),
            Some(b'-') => (LineKind::Removed, &line[1..]),
            Some(b'\\') => (LineKind::Marker, line),
            Some(_) => (LineKind::Context, &line[1..]),
            None => (LineKind::Context, line),
        };
        let (old_number, new_number) = match kind {
            LineKind::Context => {
                old += 1;
                new += 1;
                (Some(old - 1), Some(new - 1))
            }
            LineKind::Removed => {
                old += 1;
                (Some(old - 1), None)
            }
            LineKind::Added => {
                new += 1;
                (None, Some(new - 1))
            }
            LineKind::Marker => (None, None),
        };
        hunk.lines.push(DiffLine {
            kind,
            text: String::from_utf8_lossy(text).into_owned(),
            old: old_number,
            new: new_number,
        });
    }
    diff
}

/// The first old and new line numbers from `@@ -a,b +c,d @@`.
fn hunk_starts(header: &str) -> (u32, u32) {
    let mut parts = header.split_whitespace().skip(1);
    let start = |part: Option<&str>, sign: char| {
        part.and_then(|part| part.strip_prefix(sign))
            .and_then(|range| range.split(',').next())
            .and_then(|start| start.parse().ok())
            .unwrap_or(0)
    };
    let old = start(parts.next(), '-');
    let new = start(parts.next(), '+');
    (old, new)
}

/// Options that make `git diff` output parseable and appliable regardless of user config.
const DIFF_FLAGS: [&str; 5] = [
    "--no-color",
    "--no-ext-diff",
    "--src-prefix=a/",
    "--dst-prefix=b/",
    "-U3",
];

/// The diff for one entry: index → worktree when unstaged, HEAD → index when staged.
pub fn diff_for(repo: &Path, change: &FileChange) -> Result<FileDiff, String> {
    let path = checked_path(&change.path)?;
    let mut arguments = vec!["diff"];
    arguments.extend(DIFF_FLAGS);
    let mut allowed_codes: &[i32] = &[];
    if change.state == FileState::Untracked {
        arguments.extend(["--no-index", "--", "/dev/null", path]);
        allowed_codes = &[1];
    } else if change.staged {
        arguments.extend(["--cached", "-M", "--"]);
        if let Some(original) = &change.original {
            arguments.push(checked_path(original)?);
        }
        arguments.push(path);
    } else {
        arguments.extend(["--", path]);
    }
    let output = run_git(
        repo,
        &arguments,
        GitOptions {
            allowed_codes,
            ..GitOptions::default()
        },
    )?;
    Ok(parse_diff(&output))
}

/// Stages the whole file, including deletions and untracked files.
pub fn stage_file(repo: &Path, change: &FileChange) -> Result<(), String> {
    git_change(repo, &["add", "-A", "--", checked_path(&change.path)?])
}

/// Removes the whole file from the index, keeping the working copy.
pub fn unstage_file(repo: &Path, change: &FileChange, unborn: bool) -> Result<(), String> {
    let path = checked_path(&change.path)?;
    if unborn {
        return git_change(repo, &["rm", "--cached", "-r", "-q", "--", path]);
    }
    let mut arguments = vec!["restore", "--staged", "--", path];
    if let Some(original) = &change.original {
        arguments.push(checked_path(original)?);
    }
    git_change(repo, &arguments)
}

/// Applies a one-hunk patch to the index (`reverse` unstages it).
pub fn apply_to_index(repo: &Path, patch: &[u8], reverse: bool) -> Result<(), String> {
    let mut arguments = vec!["apply", "--cached", "--whitespace=nowarn"];
    if reverse {
        arguments.push("--reverse");
    }
    arguments.push("-");
    apply_patch(repo, &arguments, patch)
}

/// Removes one unstaged hunk from the working copy after snapshotting the file.
/// Returns the snapshot ref.
pub fn discard_hunk(repo: &Path, change: &FileChange, patch: &[u8]) -> Result<String, String> {
    let snapshot = snapshot(repo, &change.path, "discard hunk")?;
    apply_patch(
        repo,
        &["apply", "--reverse", "--whitespace=nowarn", "-"],
        patch,
    )?;
    Ok(snapshot)
}

/// Throws away the file's unstaged changes (deleting it when untracked) after
/// snapshotting it. Returns the snapshot ref.
pub fn discard_file(repo: &Path, change: &FileChange) -> Result<String, String> {
    let path = checked_path(&change.path)?;
    let snapshot = snapshot(repo, path, "discard file")?;
    if change.state == FileState::Untracked {
        fs::remove_file(repo.join(path))
            .map_err(|error| format!("Could not delete {path}: {error}"))?;
    } else {
        git_change(repo, &["restore", "--worktree", "--", path])?;
    }
    Ok(snapshot)
}

/// Commits the current working-copy content of `path` (on top of HEAD) to a
/// Forge-owned ref without touching the real index, so it can be restored with
/// `git checkout <ref> -- <path>`.
pub fn snapshot(repo: &Path, path: &str, label: &str) -> Result<String, String> {
    let path = checked_path(path)?;
    let index = git_path(repo, "forge-snapshot.index")?;
    let _ = fs::remove_file(&index);
    let index_text = index.to_string_lossy().into_owned();
    let env = [
        ("GIT_INDEX_FILE", index_text.as_str()),
        ("GIT_AUTHOR_NAME", "Forge"),
        ("GIT_AUTHOR_EMAIL", "forge@localhost"),
        ("GIT_COMMITTER_NAME", "Forge"),
        ("GIT_COMMITTER_EMAIL", "forge@localhost"),
    ];
    let with_env = || GitOptions {
        env: &env,
        ..GitOptions::default()
    };
    let result = (|| -> Result<String, String> {
        let head = run_git(
            repo,
            &["rev-parse", "--verify", "--quiet", "HEAD^{commit}"],
            GitOptions {
                allowed_codes: &[1],
                ..GitOptions::default()
            },
        )?;
        let head = String::from_utf8_lossy(&head).trim().to_owned();
        if !head.is_empty() {
            run_git(repo, &["read-tree", &head], with_env())?;
        }
        run_git(repo, &["add", "-A", "-f", "--", path], with_env())?;
        let tree = run_git(repo, &["write-tree"], with_env())?;
        let tree = String::from_utf8_lossy(&tree).trim().to_owned();
        let message = format!("forge snapshot: {label} {path}");
        let mut arguments = vec!["commit-tree", tree.as_str(), "-m", message.as_str()];
        if !head.is_empty() {
            arguments.extend(["-p", head.as_str()]);
        }
        let commit = run_git(repo, &arguments, with_env())?;
        let commit = String::from_utf8_lossy(&commit).trim().to_owned();
        let name = format!("{SNAPSHOT_NAMESPACE}/{}", unix_millis());
        run_git(repo, &["update-ref", &name, &commit], GitOptions::default())?;
        Ok(name)
    })();
    let _ = fs::remove_file(&index);
    result.map_err(|detail| format!("Could not save a recovery snapshot: {detail}"))
}

fn apply_patch(repo: &Path, arguments: &[&str], patch: &[u8]) -> Result<(), String> {
    run_git(
        repo,
        arguments,
        GitOptions {
            input: Some(patch),
            ..GitOptions::default()
        },
    )
    .map(|_| ())
}

fn git_change(repo: &Path, arguments: &[&str]) -> Result<(), String> {
    run_git(repo, arguments, GitOptions::default()).map(|_| ())
}

/// A file inside the repository's git directory, e.g. `FETCH_HEAD`.
pub fn git_path(repo: &Path, name: &str) -> Result<PathBuf, String> {
    let output = git_output(repo, &["rev-parse", "--git-path", name])?;
    let path = PathBuf::from(String::from_utf8_lossy(&output).trim());
    Ok(if path.is_absolute() {
        path
    } else {
        repo.join(path)
    })
}

/// Rejects paths that could escape the repository before they reach git or the file system.
fn checked_path(path: &str) -> Result<&str, String> {
    let escapes = Path::new(path)
        .components()
        .any(|part| !matches!(part, Component::Normal(_) | Component::CurDir));
    if path.is_empty() || escapes {
        Err(format!("Refusing to touch unexpected path {path:?}."))
    } else {
        Ok(path)
    }
}

/// A snapshot name stamp: the current time in milliseconds, nudged forward when needed so
/// two snapshots taken in the same millisecond (a multi-file replace) never share a ref.
fn unix_millis() -> u128 {
    static LAST: AtomicU64 = AtomicU64::new(0);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as u64);
    let mut last = LAST.load(Ordering::Relaxed);
    loop {
        let next = now.max(last + 1);
        match LAST.compare_exchange(last, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return u128::from(next),
            Err(actual) => last = actual,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{TestDirectory, git_text, run_git as git};

    #[test]
    fn snapshots_taken_back_to_back_never_share_a_ref() {
        let (_directory, repo) = repository();
        fs::write(
            repo.join("a.txt"),
            "one
",
        )
        .unwrap();
        fs::write(
            repo.join("b.txt"),
            "two
",
        )
        .unwrap();
        let mut refs = Vec::new();
        for path in ["a.txt", "b.txt", "a.txt", "b.txt", "a.txt"] {
            refs.push(snapshot(&repo, path, "test").expect("snapshot saved"));
        }
        let mut unique = refs.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), refs.len(), "{refs:?}");
        let listed = git_text(&repo, &["for-each-ref", "refs/forge/snapshots"]);
        assert_eq!(listed.lines().count(), refs.len());
    }

    #[test]
    fn parses_porcelain_status_into_staged_and_unstaged() {
        let output = b"## main...origin/main [ahead 2]\0M  staged.rs\0 M worktree.rs\0MM both.rs\0R  new.rs\0old.rs\0?? fresh.txt\0UU clash.rs\0";
        let (tracking, staged, unstaged) = parse_status(output);
        assert_eq!(tracking.branch, "main");
        assert_eq!(tracking.ahead, 2);
        let names = |list: &[FileChange]| {
            list.iter()
                .map(|change| (change.path.clone(), change.state.symbol()))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            names(&staged),
            vec![
                ("staged.rs".to_owned(), 'M'),
                ("both.rs".to_owned(), 'M'),
                ("new.rs".to_owned(), 'R'),
            ]
        );
        assert_eq!(staged[2].original.as_deref(), Some("old.rs"));
        assert_eq!(
            names(&unstaged),
            vec![
                ("worktree.rs".to_owned(), 'M'),
                ("both.rs".to_owned(), 'M'),
                ("fresh.txt".to_owned(), '?'),
                ("clash.rs".to_owned(), 'U'),
            ]
        );
    }

    #[test]
    fn parses_numstat_including_renames_and_binaries() {
        let counts = parse_numstat(b"3\t1\tsrc/a.rs\0-\t-\timage.png\x002\t0\t\0old.rs\0new.rs\0");
        assert_eq!(counts["src/a.rs"], (Some(3), Some(1)));
        assert_eq!(counts["image.png"], (None, None));
        assert_eq!(counts["new.rs"], (Some(2), Some(0)));
    }

    #[test]
    fn parses_hunks_with_line_numbers_and_rebuilds_single_hunk_patches() {
        let diff = parse_diff(
            b"diff --git a/f b/f\nindex 1..2 100644\n--- a/f\n+++ b/f\n@@ -1,2 +1,2 @@ fn x\n a\n-b\n+c\n@@ -10 +10,2 @@\n z\n+y\n\\ No newline at end of file\n",
        );
        assert_eq!(diff.hunks.len(), 2);
        let first = &diff.hunks[0];
        assert_eq!(first.header, "@@ -1,2 +1,2 @@ fn x");
        assert_eq!((first.added(), first.removed()), (1, 1));
        assert_eq!(first.lines[1].old, Some(2));
        assert_eq!(first.lines[2].new, Some(2));
        assert_eq!(diff.hunks[1].lines[1].new, Some(11));
        assert_eq!(diff.hunks[1].lines[2].kind, LineKind::Marker);
        let patch = String::from_utf8(diff.hunk_patch(1).expect("hunk exists")).unwrap();
        assert!(patch.starts_with("diff --git a/f b/f\n"));
        assert!(patch.contains("@@ -10 +10,2 @@\n z\n+y\n\\ No newline"));
        assert!(!patch.contains("-b\n"));
    }

    #[test]
    fn rejects_paths_that_escape_the_repository() {
        assert!(checked_path("src/main.rs").is_ok());
        assert!(checked_path("../outside").is_err());
        assert!(checked_path("/etc/passwd").is_err());
        assert!(checked_path("").is_err());
    }

    /// A repository with a committed ten-line file, so edits at both ends form two hunks.
    fn repository() -> (TestDirectory, PathBuf) {
        let directory = TestDirectory::new("changes");
        let repo = directory.path().join("repo");
        fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q"]);
        git(&repo, &["config", "user.name", "Forge Test"]);
        git(&repo, &["config", "user.email", "forge@example.test"]);
        git(&repo, &["config", "core.autocrlf", "false"]);
        fs::write(repo.join("file.txt"), numbered(1..=10)).unwrap();
        git(&repo, &["add", "file.txt"]);
        git(&repo, &["commit", "-q", "-m", "base"]);
        (directory, repo)
    }

    fn numbered(range: std::ops::RangeInclusive<u32>) -> String {
        range.map(|n| format!("line {n}\n")).collect()
    }

    fn edit_both_ends(repo: &Path) {
        let text = numbered(1..=10)
            .replace("line 1\n", "line one\n")
            .replace("line 10\n", "line ten\n");
        fs::write(repo.join("file.txt"), text).unwrap();
    }

    fn unstaged_file(tree: &WorkingTree) -> FileChange {
        tree.unstaged
            .iter()
            .find(|change| change.path == "file.txt")
            .cloned()
            .expect("file.txt is unstaged")
    }

    #[test]
    fn reads_working_tree_with_counts_for_every_kind_of_change() {
        let (_directory, repo) = repository();
        edit_both_ends(&repo);
        fs::write(repo.join("new.txt"), "a\nb\n").unwrap();
        fs::write(repo.join("staged.txt"), "s\n").unwrap();
        git(&repo, &["add", "staged.txt"]);

        let tree = read_working_tree(&repo).expect("tree reads");
        assert_eq!(tree.staged.len(), 1);
        assert_eq!(tree.staged[0].state, FileState::Added);
        assert_eq!(tree.staged[0].added, Some(1));
        let file = unstaged_file(&tree);
        assert_eq!((file.added, file.removed), (Some(2), Some(2)));
        let untracked = tree
            .unstaged
            .iter()
            .find(|change| change.path == "new.txt")
            .expect("untracked listed");
        assert_eq!(untracked.state, FileState::Untracked);
        assert_eq!(untracked.added, Some(2));
        assert_eq!(tree.changed_files(), 3);
        assert_eq!(tree.branches, 1);
        assert!(tree.last_fetch.is_none());
    }

    #[test]
    fn stages_and_unstages_a_single_hunk() {
        let (_directory, repo) = repository();
        edit_both_ends(&repo);
        let file = unstaged_file(&read_working_tree(&repo).unwrap());
        let diff = diff_for(&repo, &file).unwrap();
        assert_eq!(diff.hunks.len(), 2);

        apply_to_index(&repo, &diff.hunk_patch(0).unwrap(), false).expect("hunk stages");
        let cached = git_text(&repo, &["diff", "--cached"]);
        assert!(cached.contains("+line one"));
        assert!(!cached.contains("+line ten"));
        assert!(git_text(&repo, &["diff"]).contains("+line ten"));

        let tree = read_working_tree(&repo).unwrap();
        let staged = tree.staged[0].clone();
        let staged_diff = diff_for(&repo, &staged).unwrap();
        assert_eq!(staged_diff.hunks.len(), 1);
        apply_to_index(&repo, &staged_diff.hunk_patch(0).unwrap(), true).expect("hunk unstages");
        assert!(git_text(&repo, &["diff", "--cached"]).is_empty());
        assert_eq!(
            fs::read_to_string(repo.join("file.txt")).unwrap(),
            numbered(1..=10)
                .replace("line 1\n", "line one\n")
                .replace("line 10\n", "line ten\n")
        );
    }

    #[test]
    fn stages_and_unstages_whole_files_including_in_an_unborn_repository() {
        let directory = TestDirectory::new("unborn");
        let repo = directory.path().join("repo");
        fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q"]);
        fs::write(repo.join("first.txt"), "hello\n").unwrap();

        let tree = read_working_tree(&repo).unwrap();
        assert!(tree.tracking.unborn);
        stage_file(&repo, &tree.unstaged[0]).unwrap();
        let tree = read_working_tree(&repo).unwrap();
        assert_eq!(tree.staged.len(), 1);
        assert!(tree.unstaged.is_empty());

        unstage_file(&repo, &tree.staged[0], true).unwrap();
        let tree = read_working_tree(&repo).unwrap();
        assert!(tree.staged.is_empty());
        assert_eq!(tree.unstaged[0].state, FileState::Untracked);
    }

    #[test]
    fn untracked_file_diff_shows_every_line_as_added() {
        let (_directory, repo) = repository();
        fs::write(repo.join("new.txt"), "a\nb\n").unwrap();
        let tree = read_working_tree(&repo).unwrap();
        let diff = diff_for(&repo, &tree.unstaged[0]).unwrap();
        assert_eq!(diff.hunks.len(), 1);
        assert_eq!(diff.hunks[0].added(), 2);
    }

    #[test]
    fn discarding_a_hunk_snapshots_first_and_the_snapshot_restores_it() {
        let (_directory, repo) = repository();
        edit_both_ends(&repo);
        let edited = fs::read_to_string(repo.join("file.txt")).unwrap();
        let file = unstaged_file(&read_working_tree(&repo).unwrap());
        let diff = diff_for(&repo, &file).unwrap();

        let snapshot = discard_hunk(&repo, &file, &diff.hunk_patch(1).unwrap()).unwrap();
        assert!(snapshot.starts_with("refs/forge/snapshots/"));
        let after = fs::read_to_string(repo.join("file.txt")).unwrap();
        assert!(after.contains("line one"));
        assert!(after.contains("line 10\n"));
        // The real index is untouched by the snapshot.
        assert!(git_text(&repo, &["diff", "--cached"]).is_empty());

        git(&repo, &["checkout", &snapshot, "--", "file.txt"]);
        git(&repo, &["reset", "-q", "--", "file.txt"]);
        assert_eq!(fs::read_to_string(repo.join("file.txt")).unwrap(), edited);
    }

    #[test]
    fn discarding_an_untracked_file_deletes_it_only_after_a_restorable_snapshot() {
        let (_directory, repo) = repository();
        fs::write(repo.join("notes.txt"), "keep me\n").unwrap();
        let tree = read_working_tree(&repo).unwrap();

        let snapshot = discard_file(&repo, &tree.unstaged[0]).unwrap();
        assert!(!repo.join("notes.txt").exists());
        git(&repo, &["checkout", &snapshot, "--", "notes.txt"]);
        assert_eq!(
            fs::read_to_string(repo.join("notes.txt")).unwrap(),
            "keep me\n"
        );
    }

    #[test]
    fn discarding_a_tracked_file_keeps_its_staged_changes() {
        let (_directory, repo) = repository();
        fs::write(repo.join("file.txt"), "staged\n").unwrap();
        git(&repo, &["add", "file.txt"]);
        fs::write(repo.join("file.txt"), "staged\nworktree\n").unwrap();
        let file = unstaged_file(&read_working_tree(&repo).unwrap());

        discard_file(&repo, &file).unwrap();
        assert_eq!(
            fs::read_to_string(repo.join("file.txt")).unwrap(),
            "staged\n"
        );
        assert!(git_text(&repo, &["diff", "--cached"]).contains("+staged"));
    }

    #[test]
    fn hunks_apply_with_windows_line_endings_and_autocrlf() {
        let (_directory, repo) = repository();
        git(&repo, &["config", "core.autocrlf", "true"]);
        let crlf = |text: String| text.replace('\n', "\r\n");
        let edited = numbered(1..=10)
            .replace("line 1\n", "line one\n")
            .replace("line 10\n", "line ten\n");
        fs::write(repo.join("file.txt"), crlf(edited)).unwrap();

        let file = unstaged_file(&read_working_tree(&repo).unwrap());
        let diff = diff_for(&repo, &file).unwrap();
        apply_to_index(&repo, &diff.hunk_patch(0).unwrap(), false).expect("hunk stages");
        discard_hunk(&repo, &file, &diff.hunk_patch(1).unwrap()).expect("hunk discards");
        let after = fs::read_to_string(repo.join("file.txt")).unwrap();
        assert!(after.starts_with("line one\r\n"));
        assert!(after.ends_with("line 10\r\n"));
    }
}
