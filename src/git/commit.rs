//! Creating commits and pushing them.
//!
//! Hooks run exactly as `git commit` runs them (D7): Forge only lists which
//! commit hooks are installed and streams their output. Credentials come from
//! git's own helpers; terminal prompts are disabled so the TUI is never blocked.

use super::{
    changes::git_path,
    repository::{GitOptions, git_output, run_git, run_git_streaming},
};
use std::{fs, path::Path};

/// Hooks `git commit` may run, in the order it runs them.
pub const COMMIT_HOOKS: [&str; 4] = [
    "pre-commit",
    "prepare-commit-msg",
    "commit-msg",
    "post-commit",
];

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CommitInfo {
    /// Installed commit hooks, from `COMMIT_HOOKS`.
    pub hooks: Vec<String>,
    /// `commit.gpgsign` is set.
    pub sign_by_default: bool,
    /// The full message of HEAD; `None` before the first commit.
    pub last_message: Option<String>,
    /// HEAD is already on a remote-tracking branch, so amending rewrites shared history.
    pub head_pushed: bool,
    /// The repository has at least one remote to push to.
    pub has_remote: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CommitOptions {
    pub amend: bool,
    /// `Some` overrides `commit.gpgsign`; `None` leaves git's configuration alone.
    pub sign: Option<bool>,
    pub skip_hooks: bool,
}

pub fn read_commit_info(repo: &Path) -> Result<CommitInfo, String> {
    let quiet = |arguments: &[&str]| {
        run_git(
            repo,
            arguments,
            GitOptions {
                allowed_codes: &[1, 128],
                ..GitOptions::default()
            },
        )
        .map(|output| String::from_utf8_lossy(&output).trim().to_owned())
    };
    let hooks_dir = match quiet(&["config", "--get", "core.hooksPath"])? {
        configured if !configured.is_empty() => repo.join(configured),
        _ => repo.join(quiet(&["rev-parse", "--git-path", "hooks"])?),
    };
    let hooks = COMMIT_HOOKS
        .iter()
        .filter(|name| hooks_dir.join(name).is_file())
        .map(|name| (*name).to_owned())
        .collect();
    let sign_by_default = quiet(&["config", "--bool", "--get", "commit.gpgsign"])? == "true";
    let head = quiet(&["rev-parse", "--verify", "--quiet", "HEAD^{commit}"])?;
    let (last_message, head_pushed) = if head.is_empty() {
        (None, false)
    } else {
        let message = String::from_utf8_lossy(&git_output(repo, &["log", "-1", "--format=%B"])?)
            .trim_end()
            .to_owned();
        let containing = git_output(repo, &["branch", "-r", "--contains", "HEAD"])?;
        (
            Some(message),
            !containing.iter().all(u8::is_ascii_whitespace),
        )
    };
    let has_remote = !git_output(repo, &["remote"])?
        .iter()
        .all(u8::is_ascii_whitespace);
    Ok(CommitInfo {
        hooks,
        sign_by_default,
        last_message,
        head_pushed,
        has_remote,
    })
}

/// Commits the index with `message`, streaming hook and git output to `on_line`.
/// Returns `<short hash> <subject>` of the new commit.
pub fn commit(
    repo: &Path,
    message: &str,
    options: CommitOptions,
    on_line: &mut dyn FnMut(String),
) -> Result<String, String> {
    // The message goes through a file in the git directory rather than stdin so
    // hooks that read stdin cannot swallow it.
    let message_path = git_path(repo, "FORGE_COMMIT_MSG")?;
    fs::write(&message_path, message)
        .map_err(|error| format!("Could not write the commit message: {error}"))?;
    let message_arg = message_path.to_string_lossy().into_owned();
    let mut arguments = vec!["commit", "--cleanup=whitespace", "-F", message_arg.as_str()];
    if options.amend {
        arguments.push("--amend");
    }
    match options.sign {
        Some(true) => arguments.push("-S"),
        Some(false) => arguments.push("--no-gpg-sign"),
        None => {}
    }
    if options.skip_hooks {
        arguments.push("--no-verify");
    }
    let result = run_git_streaming(
        repo,
        &arguments,
        GitOptions {
            env: &[("GIT_TERMINAL_PROMPT", "0")],
            ..GitOptions::default()
        },
        on_line,
    );
    let _ = fs::remove_file(&message_path);
    result?;
    let summary = git_output(repo, &["log", "-1", "--format=%h %s"])?;
    Ok(String::from_utf8_lossy(&summary).trim().to_owned())
}

/// Pushes the current branch: to its upstream when it has one, otherwise to
/// `origin` (or the only remote) with `--set-upstream`. Returns where it went.
pub fn push(repo: &Path, on_line: &mut dyn FnMut(String)) -> Result<String, String> {
    let upstream = run_git(
        repo,
        &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"],
        GitOptions {
            allowed_codes: &[128],
            ..GitOptions::default()
        },
    )?;
    let upstream = String::from_utf8_lossy(&upstream).trim().to_owned();
    let options = || GitOptions {
        env: &[("GIT_TERMINAL_PROMPT", "0")],
        ..GitOptions::default()
    };
    if !upstream.is_empty() {
        run_git_streaming(repo, &["push", "--progress"], options(), on_line)?;
        return Ok(format!("Pushed to {upstream}."));
    }
    let remotes = String::from_utf8_lossy(&git_output(repo, &["remote"])?).into_owned();
    let remotes: Vec<&str> = remotes.lines().filter(|name| !name.is_empty()).collect();
    let remote = match remotes.as_slice() {
        [] => return Err("This repository has no remote to push to.".to_owned()),
        [only] => *only,
        many if many.contains(&"origin") => "origin",
        _ => {
            return Err(
                "The branch has no upstream and there are several remotes; push once from a shell with `git push -u <remote>`.".to_owned(),
            );
        }
    };
    run_git_streaming(
        repo,
        &["push", "--progress", "--set-upstream", remote, "HEAD"],
        options(),
        on_line,
    )?;
    Ok(format!("Pushed to {remote} and set it as upstream."))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{TestDirectory, git_text, run_git as git};
    use std::path::PathBuf;

    fn repository(directory: &TestDirectory) -> PathBuf {
        let repo = directory.path().join("repo");
        fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["config", "user.name", "Forge Test"]);
        git(&repo, &["config", "user.email", "forge@example.test"]);
        git(&repo, &["config", "commit.gpgsign", "false"]);
        repo
    }

    fn stage(repo: &Path, name: &str, content: &str) {
        fs::write(repo.join(name), content).unwrap();
        git(repo, &["add", name]);
    }

    fn install_hook(repo: &Path, name: &str, script: &str) {
        let hooks = repo.join(".git").join("hooks");
        fs::create_dir_all(&hooks).unwrap();
        fs::write(hooks.join(name), script).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(hooks.join(name), fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    #[test]
    fn reads_hooks_signing_and_head_state() {
        let directory = TestDirectory::new("commit-info");
        let repo = repository(&directory);
        let info = read_commit_info(&repo).unwrap();
        assert_eq!(info.last_message, None);
        assert!(!info.sign_by_default);
        assert!(!info.has_remote);

        install_hook(&repo, "pre-commit", "#!/bin/sh\nexit 0\n");
        install_hook(&repo, "pre-push.sample", "#!/bin/sh\n");
        stage(&repo, "a.txt", "a\n");
        git(&repo, &["commit", "-q", "-m", "first", "-m", "details"]);
        let info = read_commit_info(&repo).unwrap();
        assert_eq!(info.hooks, vec!["pre-commit".to_owned()]);
        assert_eq!(info.last_message.as_deref(), Some("first\n\ndetails"));
        assert!(!info.head_pushed);
    }

    #[test]
    fn commits_with_body_and_amends() {
        let directory = TestDirectory::new("commit");
        let repo = repository(&directory);
        stage(&repo, "a.txt", "a\n");
        let mut lines = Vec::new();
        let summary = commit(
            &repo,
            "feat: add a\n\nWhy it matters.\n\nRefs: #42\n",
            CommitOptions::default(),
            &mut |line| lines.push(line),
        )
        .unwrap();
        assert!(summary.ends_with(" feat: add a"));
        assert_eq!(
            git_text(&repo, &["log", "-1", "--format=%B"]).trim_end(),
            "feat: add a\n\nWhy it matters.\n\nRefs: #42"
        );

        commit(
            &repo,
            "feat: add a file",
            CommitOptions {
                amend: true,
                ..CommitOptions::default()
            },
            &mut |_| {},
        )
        .unwrap();
        assert_eq!(
            git_text(&repo, &["rev-list", "--count", "HEAD"]).trim(),
            "1"
        );
        assert_eq!(
            git_text(&repo, &["log", "-1", "--format=%s"]).trim(),
            "feat: add a file"
        );
        assert!(!repo.join(".git").join("FORGE_COMMIT_MSG").exists());
    }

    #[test]
    fn failing_hook_output_is_streamed_and_can_be_skipped() {
        let directory = TestDirectory::new("commit-hook");
        let repo = repository(&directory);
        install_hook(
            &repo,
            "pre-commit",
            "#!/bin/sh\necho \"lint: 2 problems in a.txt\"\nexit 1\n",
        );
        stage(&repo, "a.txt", "a\n");

        let mut lines = Vec::new();
        let error = commit(&repo, "fix: a", CommitOptions::default(), &mut |line| {
            lines.push(line)
        })
        .unwrap_err();
        assert!(lines.iter().any(|line| line.contains("lint: 2 problems")));
        assert!(error.contains("lint: 2 problems"));
        assert!(git_text(&repo, &["rev-list", "--all"]).is_empty());

        commit(
            &repo,
            "fix: a",
            CommitOptions {
                skip_hooks: true,
                ..CommitOptions::default()
            },
            &mut |_| {},
        )
        .unwrap();
        assert_eq!(
            git_text(&repo, &["log", "-1", "--format=%s"]).trim(),
            "fix: a"
        );
    }

    #[test]
    fn pushes_and_sets_upstream_then_reports_head_as_pushed() {
        let directory = TestDirectory::new("push");
        let remote = directory.path().join("remote.git");
        fs::create_dir_all(&remote).unwrap();
        git(&remote, &["init", "-q", "--bare"]);
        let repo = repository(&directory);
        git(
            &repo,
            &["remote", "add", "origin", remote.to_str().unwrap()],
        );
        stage(&repo, "a.txt", "a\n");
        commit(&repo, "first", CommitOptions::default(), &mut |_| {}).unwrap();

        let result = push(&repo, &mut |_| {}).unwrap();
        assert_eq!(result, "Pushed to origin and set it as upstream.");
        assert!(read_commit_info(&repo).unwrap().head_pushed);

        stage(&repo, "b.txt", "b\n");
        commit(&repo, "second", CommitOptions::default(), &mut |_| {}).unwrap();
        assert_eq!(push(&repo, &mut |_| {}).unwrap(), "Pushed to origin/main.");
        assert_eq!(
            git_text(&remote, &["log", "-1", "--format=%s", "main"]).trim(),
            "second"
        );
    }

    #[test]
    fn push_without_a_remote_explains_why() {
        let directory = TestDirectory::new("push-none");
        let repo = repository(&directory);
        stage(&repo, "a.txt", "a\n");
        commit(&repo, "first", CommitOptions::default(), &mut |_| {}).unwrap();
        assert_eq!(
            push(&repo, &mut |_| {}).unwrap_err(),
            "This repository has no remote to push to."
        );
    }
}
