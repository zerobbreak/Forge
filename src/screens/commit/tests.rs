use super::{
    CommitEvent, CommitExit, CommitRequest, CommitView, Field, HookRun, Phase, draw_commit,
    subject_type, trailers, with_type,
};
use crate::{
    git::{
        changes::{FileChange, FileState},
        commit::{CommitInfo, CommitOptions},
        repository::BranchTracking,
    },
    test_support::{TestDirectory, git_text, run_git},
    theme::Theme,
};
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::{Terminal, backend::TestBackend};
use std::{fs, path::PathBuf, sync::mpsc, time::Duration};

fn staged() -> Vec<FileChange> {
    ["src/services/redis.py", "src/services/cache.py"]
        .iter()
        .map(|path| FileChange {
            path: (*path).to_owned(),
            original: None,
            state: FileState::Modified,
            staged: true,
            added: Some(3),
            removed: Some(1),
        })
        .collect()
}

fn info() -> CommitInfo {
    CommitInfo {
        hooks: vec!["pre-commit".to_owned(), "commit-msg".to_owned()],
        sign_by_default: false,
        last_message: Some("feat: old subject\n\nold body".to_owned()),
        head_pushed: false,
        has_remote: true,
    }
}

fn loaded() -> CommitView {
    let tracking = BranchTracking {
        branch: "feat/redis-cache".to_owned(),
        upstream: true,
        ahead: 2,
        ..BranchTracking::default()
    };
    let mut view = CommitView::open(
        PathBuf::from("C:\\projects\\lumina"),
        staged(),
        Some(tracking),
        None,
    );
    assert_eq!(view.request.take(), Some(CommitRequest::Info));
    view.apply(CommitEvent::Info(Ok(info())));
    view
}

fn render(width: u16, height: u16, view: &CommitView) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
    terminal
        .draw(|frame| draw_commit(frame, view, &Theme::new(true), Some("unathi")))
        .expect("commit screen renders");
    let buffer = terminal.backend().buffer();
    (0..height)
        .map(|y| {
            (0..width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn press(view: &mut CommitView, code: KeyCode) {
    view.handle_key(code, KeyModifiers::NONE);
}

fn ctrl(view: &mut CommitView, character: char) {
    view.handle_key(KeyCode::Char(character), KeyModifiers::CONTROL);
}

fn type_text(view: &mut CommitView, text: &str) {
    for character in text.chars() {
        if character == '\n' {
            press(view, KeyCode::Enter);
        } else {
            press(view, KeyCode::Char(character));
        }
    }
}

#[test]
fn parses_and_rewrites_conventional_types() {
    assert_eq!(subject_type("feat(cache): move to redis"), Some("feat"));
    assert_eq!(subject_type("fix!: breaking"), Some("fix"));
    assert_eq!(subject_type("Note: not a type"), None);
    assert_eq!(subject_type("feat without colon"), None);
    assert_eq!(with_type("feat(cache): x", Some("fix")), "fix(cache): x");
    assert_eq!(
        with_type("move to redis", Some("perf")),
        "perf: move to redis"
    );
    assert_eq!(with_type("docs(readme)!: x", None), "x");
    assert_eq!(with_type("plain", None), "plain");
}

#[test]
fn reads_trailers_from_the_last_paragraph_only() {
    assert_eq!(
        trailers("Why.\n\nRefs: #42\nCo-authored-by: A <a@b.c>\n"),
        vec![
            ("Refs".to_owned(), "#42".to_owned()),
            ("Co-authored-by".to_owned(), "A <a@b.c>".to_owned()),
        ]
    );
    assert!(trailers("Refs: #42\nplain sentence").is_empty());
    assert!(trailers("").is_empty());
}

#[test]
fn renders_the_design_layout_at_80x24() {
    let mut view = loaded();
    type_text(&mut view, "feat(cache): move note cache to redis");
    press(&mut view, KeyCode::Enter);
    type_text(&mut view, "Shared between workers.\n\nRefs: #42");
    let screen = render(80, 24, &view);
    assert!(screen.contains("⎇ feat/redis-cache ↑2 ↓0"));
    assert!(screen.contains("STAGED · 2"));
    assert!(screen.contains("HOOKS"));
    assert!(screen.contains("pre-commit · runs on commit"));
    assert!(screen.contains("COMMIT MESSAGE"));
    assert!(screen.contains("[feat]"));
    assert!(screen.contains("37/72"));
    assert!(screen.contains("feat(cache): move note cache to redis"));
    assert!(screen.contains("Shared between workers."));
    assert!(screen.contains("Refs: #42"));
    assert!(screen.contains("☐ amend  ☐ sign  ☐ push  ☐ skip hooks"));
    assert!(screen.contains("ctrl+s  Commit"));
}

#[test]
fn narrow_terminals_hide_the_staged_pane_and_long_subjects_scroll() {
    let mut view = loaded();
    type_text(&mut view, &"x".repeat(80));
    let screen = render(60, 24, &view);
    assert!(!screen.contains("STAGED"));
    assert!(screen.contains("80/72"));
}

#[test]
fn letters_are_text_in_fields_and_commands_on_the_type_and_options_rows() {
    let mut view = loaded();
    type_text(&mut view, "tep");
    assert_eq!(view.subject.text(), "tep");
    assert!(!view.amend);

    press(&mut view, KeyCode::BackTab);
    assert_eq!(view.focus, Field::Type);
    press(&mut view, KeyCode::Char('t'));
    assert_eq!(view.subject.text(), "feat: tep");
    press(&mut view, KeyCode::Right);
    assert_eq!(view.subject.text(), "fix: tep");
    press(&mut view, KeyCode::Left);
    press(&mut view, KeyCode::Left);
    assert_eq!(view.subject.text(), "tep");

    press(&mut view, KeyCode::BackTab);
    assert_eq!(view.focus, Field::Options);
    press(&mut view, KeyCode::Char('p'));
    assert!(view.push_after);
    press(&mut view, KeyCode::Char(' '));
    assert!(!view.push_after);
}

#[test]
fn commit_sends_message_and_options_and_ctrl_enter_works() {
    let mut view = loaded();
    type_text(&mut view, "fix: handle empty cache");
    press(&mut view, KeyCode::Down);
    type_text(&mut view, "Body line.");
    view.handle_key(KeyCode::Enter, KeyModifiers::CONTROL);
    assert_eq!(
        view.request.take(),
        Some(CommitRequest::Commit {
            message: "fix: handle empty cache\n\nBody line.\n".to_owned(),
            options: CommitOptions::default(),
        })
    );
    assert_eq!(view.phase, Phase::Committing);
    let screen = render(80, 24, &view);
    assert!(screen.contains("◐ committing — running hooks…"));
    assert!(screen.contains("◐ pre-commit · running"));

    // Keys are ignored until git finishes.
    press(&mut view, KeyCode::Char('z'));
    assert_eq!(view.subject.text(), "fix: handle empty cache");
}

#[test]
fn validation_explains_what_to_do() {
    let mut view = loaded();
    ctrl(&mut view, 's');
    assert_eq!(view.request, None);
    assert!(render(100, 24, &view).contains("There is no subject."));

    let mut empty = CommitView::open(PathBuf::from("C:\\r"), Vec::new(), None, None);
    empty.request = None;
    empty.apply(CommitEvent::Info(Ok(info())));
    type_text(&mut empty, "fix: x");
    ctrl(&mut empty, 's');
    assert_eq!(empty.request, None);
    assert!(render(100, 24, &empty).contains("Nothing is staged."));

    let mut local = loaded();
    local.apply(CommitEvent::Info(Ok(CommitInfo {
        has_remote: false,
        ..info()
    })));
    local.push_after = true;
    type_text(&mut local, "fix: x");
    ctrl(&mut local, 's');
    assert_eq!(local.request, None);
    assert!(render(120, 24, &local).contains("no remote"));
}

#[test]
fn failed_hooks_keep_the_message_and_show_output() {
    let mut view = loaded();
    type_text(&mut view, "fix: x");
    ctrl(&mut view, 's');
    view.request = None;
    view.apply(CommitEvent::Line("ruff: 2 problems in cache.py".to_owned()));
    view.apply(CommitEvent::Committed(Err(
        "ruff: 2 problems in cache.py".to_owned()
    )));
    assert_eq!(view.phase, Phase::Editing);
    assert_eq!(view.hooks, HookRun::Failed);
    assert_eq!(view.subject.text(), "fix: x");
    let screen = render(120, 30, &view);
    assert!(screen.contains("✕ pre-commit · failed"));
    assert!(screen.contains("ruff: 2 problems in cache.py"));
    assert!(screen.contains("✕ Commit failed:"));
}

#[test]
fn successful_commit_exits_or_pushes_first() {
    let mut view = loaded();
    type_text(&mut view, "fix: x");
    ctrl(&mut view, 's');
    view.apply(CommitEvent::Committed(Ok("abc1234 fix: x".to_owned())));
    assert!(matches!(
        &view.exit,
        Some(CommitExit::Done(notice)) if notice.text == "Committed abc1234 fix: x." && !notice.error
    ));

    let mut view = loaded();
    view.push_after = true;
    type_text(&mut view, "fix: x");
    ctrl(&mut view, 's');
    view.request = None;
    view.apply(CommitEvent::Committed(Ok("abc1234 fix: x".to_owned())));
    assert_eq!(view.request, Some(CommitRequest::Push));
    assert!(render(80, 24, &view).contains("◐ pushing…"));
    view.apply(CommitEvent::Pushed(Err("Authentication failed".to_owned())));
    assert!(matches!(
        &view.exit,
        Some(CommitExit::Done(notice)) if notice.error && notice.text.contains("but the push failed: Authentication failed")
    ));
}

#[test]
fn amend_loads_the_last_message_and_warns_when_pushed() {
    let mut view = loaded();
    view.apply(CommitEvent::Info(Ok(CommitInfo {
        head_pushed: true,
        ..info()
    })));
    view.focus = Field::Options;
    press(&mut view, KeyCode::Char('e'));
    assert!(view.amend);
    assert_eq!(view.subject.text(), "feat: old subject");
    assert_eq!(view.body.text(), "old body");
    assert!(render(140, 24, &view).contains("already pushed"));
    press(&mut view, KeyCode::Char('e'));
    assert!(view.subject.is_empty());

    let mut first = loaded();
    first.apply(CommitEvent::Info(Ok(CommitInfo {
        last_message: None,
        ..info()
    })));
    first.focus = Field::Options;
    press(&mut first, KeyCode::Char('e'));
    assert!(!first.amend);
}

#[test]
fn signing_follows_config_and_only_overrides_when_changed() {
    let mut view = loaded();
    view.apply(CommitEvent::Info(Ok(CommitInfo {
        sign_by_default: true,
        ..info()
    })));
    assert!(render(80, 24, &view).contains("☑ sign"));
    type_text(&mut view, "fix: x");
    ctrl(&mut view, 's');
    assert!(matches!(
        view.request.take(),
        Some(CommitRequest::Commit { options, .. }) if options.sign.is_none()
    ));

    let mut view = loaded();
    view.focus = Field::Options;
    press(&mut view, KeyCode::Char('g'));
    view.focus = Field::Subject;
    type_text(&mut view, "fix: x");
    ctrl(&mut view, 's');
    assert!(matches!(
        view.request.take(),
        Some(CommitRequest::Commit { options, .. }) if options.sign == Some(true)
    ));
}

#[test]
fn escape_cancels_and_keeps_the_draft() {
    let mut view = loaded();
    type_text(&mut view, "wip");
    press(&mut view, KeyCode::Esc);
    assert_eq!(
        view.exit,
        Some(CommitExit::Cancelled {
            subject: "wip".to_owned(),
            body: String::new(),
        })
    );
    let reopened = CommitView::open(
        PathBuf::from("C:\\r"),
        staged(),
        None,
        Some(("wip".to_owned(), "more".to_owned())),
    );
    assert_eq!(reopened.subject.text(), "wip");
    assert_eq!(reopened.body.text(), "more");
}

/// Commits a staged file through the real background request path.
#[test]
fn commits_end_to_end_through_background_requests() {
    let directory = TestDirectory::new("commit-screen");
    let repo = directory.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    run_git(&repo, &["init", "-q"]);
    run_git(&repo, &["config", "user.name", "Forge Test"]);
    run_git(&repo, &["config", "user.email", "forge@example.test"]);
    run_git(&repo, &["config", "commit.gpgsign", "false"]);
    fs::write(repo.join("a.txt"), "a\n").unwrap();
    run_git(&repo, &["add", "a.txt"]);

    let (sender, receiver) = mpsc::channel();
    let mut view = CommitView::open(repo.clone(), staged(), None, None);
    let pump = |view: &mut CommitView| {
        while let Some(request) = view.request.take() {
            super::spawn_request(view.path.clone(), request, sender.clone());
            loop {
                let (path, event) = receiver
                    .recv_timeout(Duration::from_secs(20))
                    .expect("background git finishes");
                assert_eq!(path, view.path);
                let done = !matches!(event, CommitEvent::Line(_));
                view.apply(event);
                if done {
                    break;
                }
            }
        }
    };
    pump(&mut view);
    assert!(view.loaded_info().is_some());
    type_text(&mut view, "feat: add a");
    ctrl(&mut view, 's');
    pump(&mut view);
    assert!(matches!(&view.exit, Some(CommitExit::Done(notice)) if !notice.error));
    assert_eq!(
        git_text(&repo, &["log", "-1", "--format=%s"]).trim(),
        "feat: add a"
    );
}
