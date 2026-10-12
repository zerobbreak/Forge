//! Running the system `git` binary and parsing its repository-level output.
//!
//! Every call passes arguments as an array (never a shell string) and runs
//! `git -C <repo>`, so repository paths and branch names stay untrusted data.

use std::{
    io::{self, BufReader, Read, Write},
    path::Path,
    process::{Child, Command, Stdio},
    sync::mpsc,
    thread::{self, JoinHandle},
};

/// Options for [`run_git`] beyond the argument list.
#[derive(Default)]
pub struct GitOptions<'a> {
    /// Bytes written to git's standard input, e.g. a patch for `git apply -`.
    pub input: Option<&'a [u8]>,
    /// Extra environment variables, e.g. `GIT_INDEX_FILE`.
    pub env: &'a [(&'a str, &'a str)],
    /// Exit codes other than 0 that still count as success (`git diff --no-index` exits 1).
    pub allowed_codes: &'a [i32],
}

/// The helper thread that writes a command's stdin.
type StdinWriter = JoinHandle<io::Result<()>>;

/// Starts `git -C path arguments…` with piped output, writing `options.input` to
/// stdin on a helper thread so large inputs cannot deadlock against output.
fn spawn_git(
    path: &Path,
    arguments: &[&str],
    options: &GitOptions<'_>,
) -> Result<(Child, Option<StdinWriter>), String> {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(path)
        .args(arguments)
        .stdin(if options.input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (name, value) in options.env {
        command.env(name, value);
    }
    let mut child = command.spawn().map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            "Git was not found on PATH.".to_owned()
        } else {
            format!("Could not run Git in {}: {error}", path.display())
        }
    })?;
    let writer = match (options.input, child.stdin.take()) {
        (Some(input), Some(mut stdin)) => {
            let input = input.to_vec();
            Some(thread::spawn(move || stdin.write_all(&input)))
        }
        _ => None,
    };
    Ok((child, writer))
}

/// Runs git like [`run_git`], passing each line of stdout and stderr to `on_line`
/// as it arrives (hook output, push progress). Progress lines ending in `\r`
/// count as lines. On failure the error holds the last few output lines.
pub fn run_git_streaming(
    path: &Path,
    arguments: &[&str],
    options: GitOptions<'_>,
    on_line: &mut dyn FnMut(String),
) -> Result<(), String> {
    const ERROR_LINES: usize = 3;
    let (mut child, writer) = spawn_git(path, arguments, &options)?;
    let (sender, receiver) = mpsc::channel();
    let readers: Vec<_> = [
        child
            .stdout
            .take()
            .map(|out| Box::new(out) as Box<dyn Read + Send>),
        child
            .stderr
            .take()
            .map(|err| Box::new(err) as Box<dyn Read + Send>),
    ]
    .into_iter()
    .flatten()
    .map(|stream| {
        let sender = sender.clone();
        thread::spawn(move || {
            let mut reader = BufReader::new(stream);
            let mut line = Vec::new();
            let mut byte = [0u8; 1];
            while matches!(reader.read(&mut byte), Ok(1)) {
                if byte[0] == b'\n' || byte[0] == b'\r' {
                    if !line.is_empty() {
                        let _ = sender.send(String::from_utf8_lossy(&line).into_owned());
                        line.clear();
                    }
                } else {
                    line.push(byte[0]);
                }
            }
            if !line.is_empty() {
                let _ = sender.send(String::from_utf8_lossy(&line).into_owned());
            }
        })
    })
    .collect();
    drop(sender);
    let mut recent = Vec::new();
    for line in receiver {
        recent.push(line.clone());
        if recent.len() > ERROR_LINES {
            recent.remove(0);
        }
        on_line(line);
    }
    for reader in readers {
        let _ = reader.join();
    }
    if let Some(writer) = writer {
        let _ = writer.join();
    }
    let status = child
        .wait()
        .map_err(|error| format!("Could not run Git in {}: {error}", path.display()))?;
    if status.success() {
        return Ok(());
    }
    let command = arguments.first().copied().unwrap_or("git");
    Err(if recent.is_empty() {
        format!("`git {command}` failed in {}.", path.display())
    } else {
        recent.join(" · ")
    })
}

/// Runs `git -C path arguments…` and returns its standard output.
pub fn run_git(
    path: &Path,
    arguments: &[&str],
    options: GitOptions<'_>,
) -> Result<Vec<u8>, String> {
    let (child, writer) = spawn_git(path, arguments, &options)?;
    let output = child
        .wait_with_output()
        .map_err(|error| format!("Could not run Git in {}: {error}", path.display()))?;
    if let Some(writer) = writer {
        let _ = writer.join();
    }
    let code = output.status.code();
    if output.status.success() || code.is_some_and(|code| options.allowed_codes.contains(&code)) {
        return Ok(output.stdout);
    }
    let detail = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    let command = arguments.first().copied().unwrap_or("git");
    Err(if detail.is_empty() {
        format!("`git {command}` failed in {}.", path.display())
    } else {
        detail
    })
}

/// Read-only `git` query; errors read "Could not inspect <path>: …".
pub fn git_output(path: &Path, arguments: &[&str]) -> Result<Vec<u8>, String> {
    run_git(path, arguments, GitOptions::default()).map_err(|detail| {
        if detail.starts_with("Git was not found") || detail.starts_with("Could not run Git") {
            detail
        } else {
            format!("Could not inspect {}: {detail}", path.display())
        }
    })
}

/// A `git config --get`-style query: `Ok(None)` when git exits 1 (unset).
pub fn git_optional_output(path: &Path, arguments: &[&str]) -> Result<Option<String>, String> {
    let output = run_git(
        path,
        arguments,
        GitOptions {
            allowed_codes: &[1],
            ..GitOptions::default()
        },
    )
    .map_err(|detail| format!("Could not inspect {}: {detail}", path.display()))?;
    let value = String::from_utf8_lossy(&output).trim().to_owned();
    Ok((!value.is_empty()).then(|| sanitize_remote(&value)))
}

/// Removes credentials from a remote URL before it is displayed.
pub fn sanitize_remote(remote: &str) -> String {
    if let Some((scheme, authority_and_path)) = remote.split_once("://") {
        let (authority, path) = authority_and_path
            .split_once('/')
            .unwrap_or((authority_and_path, ""));
        let host = authority
            .rsplit_once('@')
            .map_or(authority, |(_, host)| host);
        if path.is_empty() {
            format!("{scheme}://{host}")
        } else {
            format!("{scheme}://{host}/{path}")
        }
    } else if let Some((user, host_and_path)) = remote.split_once('@')
        && !user.contains('/')
        && !user.contains('\\')
    {
        host_and_path.to_owned()
    } else {
        remote.to_owned()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BranchTracking {
    pub branch: String,
    pub upstream: bool,
    pub ahead: usize,
    pub behind: usize,
    pub unborn: bool,
}

/// Parses the `## ...` header printed by `git status --porcelain=1 --branch`.
pub fn parse_branch_line(line: &str) -> BranchTracking {
    let header = line.strip_prefix("## ").unwrap_or(line);
    if let Some(branch) = header.strip_prefix("No commits yet on ") {
        return BranchTracking {
            branch: branch.to_owned(),
            unborn: true,
            ..BranchTracking::default()
        };
    }

    let (head, counts) = header
        .split_once(" [")
        .map_or((header, ""), |(head, counts)| {
            (head, counts.trim_end_matches(']'))
        });
    let (branch, upstream) = head
        .split_once("...")
        .map_or((head, false), |(branch, _)| (branch, true));
    let mut ahead = 0;
    let mut behind = 0;
    for part in counts.split(", ") {
        if let Some(count) = part.strip_prefix("ahead ") {
            ahead = count.parse().unwrap_or(0);
        } else if let Some(count) = part.strip_prefix("behind ") {
            behind = count.parse().unwrap_or(0);
        }
    }

    BranchTracking {
        branch: branch.to_owned(),
        upstream,
        ahead,
        behind,
        unborn: false,
    }
}

pub struct Commit {
    pub hash: String,
    pub subject: String,
}

/// Parses `git log --format=%h%x09%ct%x09%s` into the newest commit time and commits.
pub fn parse_log(log: &str) -> (Option<u64>, Vec<Commit>) {
    let mut last_commit = None;
    let commits = log
        .lines()
        .filter_map(|line| {
            let mut fields = line.splitn(3, '\t');
            let hash = fields.next()?.to_owned();
            let time = fields.next()?.parse().ok();
            last_commit = last_commit.or(time);
            Some(Commit {
                hash,
                subject: fields.next().unwrap_or_default().to_owned(),
            })
        })
        .collect();
    (last_commit, commits)
}

#[cfg(test)]
mod tests {
    use super::{BranchTracking, parse_branch_line, sanitize_remote};

    #[test]
    fn parses_branch_header_tracking_counts() {
        assert_eq!(
            parse_branch_line("## feat/x...origin/feat/x [ahead 3, behind 12]"),
            BranchTracking {
                branch: "feat/x".to_owned(),
                upstream: true,
                ahead: 3,
                behind: 12,
                unborn: false,
            }
        );
        let local_only = parse_branch_line("## main");
        assert_eq!(local_only.branch, "main");
        assert!(!local_only.upstream);
        let unborn = parse_branch_line("## No commits yet on main");
        assert!(unborn.unborn);
        assert_eq!(unborn.branch, "main");
    }

    #[test]
    fn removes_credentials_from_remote_urls_before_display() {
        assert_eq!(
            sanitize_remote("https://user:secret@example.test/org/repo.git"),
            "https://example.test/org/repo.git"
        );
        assert_eq!(
            sanitize_remote("git@example.test:org/repo.git"),
            "example.test:org/repo.git"
        );
    }
}
