//! Reading commit history for the Commits screen: the graph log and one commit's detail.

use super::repository::git_output;
use std::path::Path;

/// Commits requested per page of the log.
pub const PAGE: usize = 300;

/// Which refs the log walks from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogScope {
    /// Every local branch, remote branch, tag and `HEAD`.
    All,
    /// Only the history reachable from `HEAD`.
    Head,
}

/// A ref pointing at a commit, as shown in a row's chips.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RefLabel {
    /// The checked-out branch (`HEAD -> main`), or a detached `HEAD`.
    Head(String),
    Branch(String),
    Remote(String),
    Tag(String),
}

impl RefLabel {
    pub fn name(&self) -> &str {
        match self {
            Self::Head(name) | Self::Branch(name) | Self::Remote(name) | Self::Tag(name) => name,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitInfo {
    pub hash: String,
    pub short: String,
    pub author: String,
    pub time: u64,
    pub refs: Vec<RefLabel>,
    pub subject: String,
}

/// One line of `git log --graph`: the drawn lanes, and the commit when this line has one.
/// Lines without a commit only connect lanes (`|\`, `|/`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogRow {
    pub graph: String,
    pub commit: Option<CommitInfo>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogPage {
    pub rows: Vec<LogRow>,
    /// The page was full, so older commits probably exist.
    pub more: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileStat {
    pub path: String,
    /// `None` for binary files.
    pub added: Option<usize>,
    pub removed: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitDetail {
    pub hash: String,
    pub author: String,
    pub email: String,
    pub time: u64,
    pub parents: Vec<String>,
    pub message: String,
    pub files: Vec<FileStat>,
}

const RECORD: char = '\u{1e}';
const FIELD: char = '\u{1f}';

/// Reads up to `limit` commits with their graph lanes.
pub fn read_log(repo: &Path, scope: LogScope, limit: usize) -> Result<LogPage, String> {
    let count = format!("-n{limit}");
    let format = format!("--format={RECORD}%H{FIELD}%h{FIELD}%an{FIELD}%ct{FIELD}%D{FIELD}%s");
    let mut arguments = vec![
        "log",
        "--graph",
        "--no-color",
        "--decorate=full",
        &count,
        &format,
    ];
    match scope {
        LogScope::All => arguments.extend(["--branches", "--remotes", "--tags", "HEAD"]),
        LogScope::Head => arguments.push("HEAD"),
    }
    let output = match git_output(repo, &arguments) {
        Ok(output) => output,
        // A repository with no commits has no history yet; that is not an error.
        Err(error) if is_unborn(&error) => {
            return Ok(LogPage {
                rows: Vec::new(),
                more: false,
            });
        }
        Err(error) => return Err(error),
    };
    let rows = parse_log(&String::from_utf8_lossy(&output));
    let commits = rows.iter().filter(|row| row.commit.is_some()).count();
    Ok(LogPage {
        rows,
        more: commits >= limit,
    })
}

fn is_unborn(error: &str) -> bool {
    error.contains("does not have any commits yet")
        || error.contains("bad default revision")
        || error.contains("ambiguous argument 'HEAD'")
}

/// Parses `git log --graph` output printed with [`read_log`]'s format.
pub fn parse_log(output: &str) -> Vec<LogRow> {
    output
        .lines()
        .filter_map(|line| {
            let Some((graph, record)) = line.split_once(RECORD) else {
                let graph = line.trim_end();
                return (!graph.is_empty()).then(|| LogRow {
                    graph: graph.to_owned(),
                    commit: None,
                });
            };
            let mut fields = record.splitn(6, FIELD);
            let hash = fields.next()?.to_owned();
            let short = fields.next()?.to_owned();
            let author = fields.next()?.to_owned();
            let time = fields.next()?.parse().unwrap_or(0);
            let refs = parse_refs(fields.next()?);
            let subject = fields.next().unwrap_or_default().to_owned();
            Some(LogRow {
                graph: graph.to_owned(),
                commit: Some(CommitInfo {
                    hash,
                    short,
                    author,
                    time,
                    refs,
                    subject,
                }),
            })
        })
        .collect()
}

/// Parses `%D` printed with `--decorate=full`, e.g.
/// `HEAD -> refs/heads/main, tag: refs/tags/v1, refs/remotes/origin/main`.
fn parse_refs(decoration: &str) -> Vec<RefLabel> {
    decoration
        .split(", ")
        .filter(|part| !part.is_empty())
        .filter_map(|part| {
            if let Some(branch) = part.strip_prefix("HEAD -> ") {
                let name = branch.strip_prefix("refs/heads/").unwrap_or(branch);
                return Some(RefLabel::Head(name.to_owned()));
            }
            if part == "HEAD" {
                return Some(RefLabel::Head("HEAD".to_owned()));
            }
            if let Some(tag) = part.strip_prefix("tag: ") {
                let name = tag.strip_prefix("refs/tags/").unwrap_or(tag);
                return Some(RefLabel::Tag(name.to_owned()));
            }
            if let Some(branch) = part.strip_prefix("refs/heads/") {
                return Some(RefLabel::Branch(branch.to_owned()));
            }
            if let Some(remote) = part.strip_prefix("refs/remotes/") {
                // `origin/HEAD` is a pointer, not a branch worth a chip.
                return (!remote.ends_with("/HEAD")).then(|| RefLabel::Remote(remote.to_owned()));
            }
            None
        })
        .collect()
}

/// Reads one commit's author, message and changed files.
pub fn read_commit_detail(repo: &Path, hash: &str) -> Result<CommitDetail, String> {
    // The hash comes from our own log output, but it ends up on a command line.
    if hash.is_empty() || !hash.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!("`{hash}` is not a commit id."));
    }
    let format = format!("--format=%H{FIELD}%an{FIELD}%ae{FIELD}%ct{FIELD}%P{FIELD}%B");
    let header = git_output(repo, &["show", "-s", "--no-color", &format, hash])?;
    let header = String::from_utf8_lossy(&header);
    let mut fields = header.splitn(6, FIELD);
    let mut next = || fields.next().unwrap_or_default().to_owned();
    let (full, author, email, time, parents, message) =
        (next(), next(), next(), next(), next(), next());
    let parents: Vec<String> = parents.split_whitespace().map(str::to_owned).collect();

    // A merge has no diff of its own; show what it brought in over its first parent.
    let numstat = match parents.first() {
        Some(parent) if parents.len() > 1 => git_output(
            repo,
            &[
                "-c",
                "core.quotepath=false",
                "diff-tree",
                "--numstat",
                "-r",
                "-M",
                parent,
                hash,
            ],
        )?,
        _ => git_output(
            repo,
            &[
                "-c",
                "core.quotepath=false",
                "diff-tree",
                "--numstat",
                "-r",
                "-M",
                "--root",
                "--no-commit-id",
                hash,
            ],
        )?,
    };
    Ok(CommitDetail {
        hash: full.trim().to_owned(),
        author,
        email,
        time: time.trim().parse().unwrap_or(0),
        parents,
        message: message.trim().to_owned(),
        files: parse_numstat(&String::from_utf8_lossy(&numstat)),
    })
}

fn parse_numstat(output: &str) -> Vec<FileStat> {
    output
        .lines()
        .filter_map(|line| {
            let mut fields = line.splitn(3, '\t');
            let added = fields.next()?;
            let removed = fields.next()?;
            let path = fields.next()?;
            Some(FileStat {
                path: path.to_owned(),
                added: added.parse().ok(),
                removed: removed.parse().ok(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{LogScope, RefLabel, parse_log, parse_refs, read_commit_detail, read_log};
    use crate::test_support::{TestDirectory, git_text, run_git};
    use std::{fs, path::PathBuf};

    fn repository(directory: &TestDirectory) -> PathBuf {
        let repo = directory.path().join("repo");
        fs::create_dir_all(&repo).unwrap();
        run_git(&repo, &["init", "-b", "main"]);
        run_git(&repo, &["config", "user.name", "Forge Test"]);
        run_git(&repo, &["config", "user.email", "forge@example.test"]);
        run_git(&repo, &["config", "commit.gpgsign", "false"]);
        repo
    }

    fn commit(repo: &PathBuf, file: &str, text: &str, message: &str) {
        fs::write(repo.join(file), text).unwrap();
        run_git(repo, &["add", file]);
        run_git(repo, &["commit", "-m", message]);
    }

    #[test]
    fn parses_decorations_into_labelled_refs() {
        assert_eq!(
            parse_refs(
                "HEAD -> refs/heads/main, tag: refs/tags/v1.0, refs/remotes/origin/main, refs/remotes/origin/HEAD, refs/heads/feat/x"
            ),
            vec![
                RefLabel::Head("main".to_owned()),
                RefLabel::Tag("v1.0".to_owned()),
                RefLabel::Remote("origin/main".to_owned()),
                RefLabel::Branch("feat/x".to_owned()),
            ]
        );
        assert_eq!(
            parse_refs("HEAD"),
            vec![RefLabel::Head("HEAD".to_owned())]
        );
        assert!(parse_refs("").is_empty());
    }

    #[test]
    fn separates_graph_lanes_from_commit_fields() {
        let output = "* \u{1e}aaa\u{1f}aa\u{1f}Ann\u{1f}100\u{1f}HEAD -> refs/heads/main\u{1f}Merge x\n|\\  \n| * \u{1e}bbb\u{1f}bb\u{1f}Bo\u{1f}90\u{1f}\u{1f}Side: work\n|/  \n";
        let rows = parse_log(output);
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0].graph, "* ");
        let first = rows[0].commit.as_ref().unwrap();
        assert_eq!((first.short.as_str(), first.author.as_str()), ("aa", "Ann"));
        assert_eq!(first.refs, vec![RefLabel::Head("main".to_owned())]);
        assert!(rows[1].commit.is_none());
        assert_eq!(rows[1].graph, "|\\");
        assert_eq!(rows[2].commit.as_ref().unwrap().subject, "Side: work");
        assert_eq!(rows[2].graph, "| * ");
    }

    #[test]
    fn reads_a_branched_history_with_refs() {
        let directory = TestDirectory::new("log");
        let repo = repository(&directory);
        commit(&repo, "a.txt", "1\n", "feat: first");
        run_git(&repo, &["tag", "v0.1"]);
        run_git(&repo, &["switch", "-c", "feat/side"]);
        commit(&repo, "b.txt", "2\n", "feat: side work");
        run_git(&repo, &["switch", "main"]);
        commit(&repo, "c.txt", "3\n", "fix: main work");
        run_git(&repo, &["merge", "--no-ff", "feat/side", "-m", "Merge feat/side"]);

        let page = read_log(&repo, LogScope::All, 50).expect("log reads");
        let commits: Vec<_> = page.rows.iter().filter_map(|row| row.commit.as_ref()).collect();
        let subjects: Vec<_> = commits.iter().map(|c| c.subject.as_str()).collect();
        assert_eq!(subjects[0], "Merge feat/side");
        assert!(subjects.contains(&"feat: side work"));
        assert!(subjects.contains(&"fix: main work"));
        assert!(subjects.contains(&"feat: first"));
        assert!(!page.more);
        // Both lanes are drawn: at least one connector-only row exists.
        assert!(page.rows.iter().any(|row| row.commit.is_none()));
        assert!(commits[0].refs.contains(&RefLabel::Head("main".to_owned())));
        let first = commits.iter().find(|c| c.subject == "feat: first").unwrap();
        assert!(first.refs.contains(&RefLabel::Tag("v0.1".to_owned())));
        let side = commits.iter().find(|c| c.subject == "feat: side work").unwrap();
        assert!(side.refs.contains(&RefLabel::Branch("feat/side".to_owned())));

        let head_only = read_log(&repo, LogScope::Head, 50).expect("log reads");
        assert!(head_only.rows.iter().filter(|r| r.commit.is_some()).count() >= 4);
    }

    #[test]
    fn marks_a_full_page_as_having_more() {
        let directory = TestDirectory::new("log-page");
        let repo = repository(&directory);
        for n in 0..3 {
            commit(&repo, "a.txt", &format!("{n}\n"), &format!("change {n}"));
        }
        assert!(read_log(&repo, LogScope::Head, 3).unwrap().more);
        assert!(!read_log(&repo, LogScope::Head, 4).unwrap().more);
    }

    #[test]
    fn a_repository_without_commits_has_an_empty_log() {
        let directory = TestDirectory::new("log-unborn");
        let repo = repository(&directory);
        for scope in [LogScope::All, LogScope::Head] {
            let page = read_log(&repo, scope, 10).expect("unborn is not an error");
            assert!(page.rows.is_empty());
        }
    }

    #[test]
    fn reads_commit_detail_with_file_counts() {
        let directory = TestDirectory::new("log-detail");
        let repo = repository(&directory);
        commit(&repo, "a.txt", "1\n2\n", "feat: first");
        fs::write(repo.join("a.txt"), "1\n2\n3\n").unwrap();
        fs::write(repo.join("img.bin"), [0u8, 159, 146, 150]).unwrap();
        run_git(&repo, &["add", "."]);
        run_git(&repo, &["commit", "-m", "feat: second\n\nBody line.\n\nRefs: #4"]);
        let hash = git_text(&repo, &["rev-parse", "HEAD"]).trim().to_owned();

        let detail = read_commit_detail(&repo, &hash).expect("detail reads");
        assert_eq!(detail.hash, hash);
        assert_eq!(detail.author, "Forge Test");
        assert_eq!(detail.email, "forge@example.test");
        assert_eq!(detail.parents.len(), 1);
        assert!(detail.message.starts_with("feat: second"));
        assert!(detail.message.contains("Refs: #4"));
        let text = detail.files.iter().find(|f| f.path == "a.txt").unwrap();
        assert_eq!((text.added, text.removed), (Some(1), Some(0)));
        let binary = detail.files.iter().find(|f| f.path == "img.bin").unwrap();
        assert_eq!(binary.added, None);
    }

    #[test]
    fn the_root_commit_lists_its_files_and_a_merge_lists_what_it_brought_in() {
        let directory = TestDirectory::new("log-root");
        let repo = repository(&directory);
        commit(&repo, "a.txt", "1\n", "feat: root");
        let root = git_text(&repo, &["rev-parse", "HEAD"]).trim().to_owned();
        assert_eq!(read_commit_detail(&repo, &root).unwrap().files.len(), 1);

        run_git(&repo, &["switch", "-c", "side"]);
        commit(&repo, "b.txt", "2\n", "feat: side");
        run_git(&repo, &["switch", "main"]);
        commit(&repo, "c.txt", "3\n", "fix: main");
        run_git(&repo, &["merge", "--no-ff", "side", "-m", "Merge side"]);
        let merge = git_text(&repo, &["rev-parse", "HEAD"]).trim().to_owned();
        let detail = read_commit_detail(&repo, &merge).unwrap();
        assert_eq!(detail.parents.len(), 2);
        assert_eq!(detail.files.len(), 1);
        assert_eq!(detail.files[0].path, "b.txt");
    }

    #[test]
    fn refuses_a_commit_id_that_is_not_hex() {
        let directory = TestDirectory::new("log-bad");
        let repo = repository(&directory);
        assert!(read_commit_detail(&repo, "--output=x").is_err());
        assert!(read_commit_detail(&repo, "").is_err());
    }
}
