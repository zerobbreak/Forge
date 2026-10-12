use super::{CommitsEvent, CommitsRequest, CommitsView, draw_commits};
use crate::{
    git::{
        log::{
            CommitDetail, CommitInfo, FileStat, LogPage, LogRow, LogScope, PAGE, RefLabel,
            read_commit_detail, read_log,
        },
        repository::BranchTracking,
    },
    test_support::{TestDirectory, git_text, run_git},
    theme::Theme,
    ui::unix_now,
};
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::{Terminal, backend::TestBackend};
use std::{fs, path::PathBuf};

fn commit(hash: &str, subject: &str, refs: Vec<RefLabel>) -> CommitInfo {
    CommitInfo {
        hash: format!("{hash}{}", "0".repeat(40 - hash.len())),
        short: hash.chars().take(7).collect(),
        author: "Unathi Tshuma".to_owned(),
        time: unix_now() - 3 * 3600,
        refs,
        subject: subject.to_owned(),
    }
}

fn row(graph: &str, commit: Option<CommitInfo>) -> LogRow {
    LogRow {
        graph: graph.to_owned(),
        commit,
    }
}

/// A merge of a side branch: commit, connector, side commit, connector, root.
fn page() -> LogPage {
    LogPage {
        rows: vec![
            row(
                "*   ",
                Some(commit(
                    "aaaaaaa",
                    "Merge feat/redis-cache",
                    vec![
                        RefLabel::Head("main".to_owned()),
                        RefLabel::Remote("origin/main".to_owned()),
                    ],
                )),
            ),
            row("|\\  ", None),
            row(
                "| * ",
                Some(commit(
                    "bbbbbbb",
                    "feat(cache): add redis client",
                    vec![RefLabel::Branch("feat/redis-cache".to_owned())],
                )),
            ),
            row(
                "* | ",
                Some(commit("ccccccc", "fix: trim whitespace", vec![])),
            ),
            row("|/  ", None),
            row(
                "*   ",
                Some(commit(
                    "ddddddd",
                    "chore: first commit",
                    vec![RefLabel::Tag("v0.1".to_owned())],
                )),
            ),
        ],
        more: false,
    }
}

fn detail(hash: &str) -> CommitDetail {
    CommitDetail {
        hash: hash.to_owned(),
        author: "Unathi Tshuma".to_owned(),
        email: "unathi@example.test".to_owned(),
        time: unix_now() - 3 * 3600,
        parents: vec!["1111111aaaa".to_owned()],
        message: "feat(cache): add redis client\n\nTalks to redis with a TTL.\n\nRefs: #42"
            .to_owned(),
        files: vec![
            FileStat {
                path: "src/services/redis.py".to_owned(),
                added: Some(12),
                removed: Some(0),
            },
            FileStat {
                path: "logo.png".to_owned(),
                added: None,
                removed: None,
            },
        ],
    }
}

fn tracking() -> BranchTracking {
    BranchTracking {
        branch: "main".to_owned(),
        upstream: true,
        ahead: 1,
        behind: 0,
        unborn: false,
    }
}

/// A view with the sample page loaded and the cursor on the first commit.
fn loaded_view() -> CommitsView {
    let mut view = CommitsView::open(PathBuf::from("C:\\projects\\lumina"), Some(tracking()));
    assert_eq!(
        view.request.take(),
        Some(CommitsRequest::Load {
            scope: LogScope::All,
            limit: PAGE
        })
    );
    view.apply(CommitsEvent::Loaded {
        scope: LogScope::All,
        limit: PAGE,
        result: Ok(page()),
    });
    let Some(CommitsRequest::Detail(hash)) = view.request.take() else {
        panic!("loading asks for the selected commit's detail");
    };
    view.apply(CommitsEvent::Detail(hash.clone(), Ok(detail(&hash))));
    view
}

fn render(width: u16, height: u16, view: &CommitsView, colors: bool) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
    terminal
        .draw(|frame| draw_commits(frame, view, &Theme::new(colors), Some("unathi")))
        .expect("commits screen renders");
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

fn key(view: &mut CommitsView, code: KeyCode) {
    view.handle_key(code, KeyModifiers::NONE);
}

#[test]
fn renders_the_graph_and_key_bar_at_80x24() {
    let view = loaded_view();
    let screen = render(80, 24, &view, true);
    assert!(screen.contains("FORGE"));
    assert!(screen.contains("⎇ main ↑1 ↓0"));
    assert!(screen.contains("COMMITS · ALL BRANCHES · 4"));
    assert!(screen.contains("> ●"), "the cursor row is marked");
    assert!(screen.contains("aaaaaaa"));
    assert!(screen.contains("[⎇ main]"));
    assert!(screen.contains("[origin/main]"));
    assert!(
        screen.contains("Merge feat/"),
        "subject is shown beside the chips"
    );
    assert!(
        screen.contains("│ ●"),
        "side lane is drawn with box characters"
    );
    assert!(screen.contains("[tag: v0.1]"));
    assert!(screen.contains("3h"));
    assert!(screen.contains("j/k Move"));
    assert!(screen.contains("a Current branch"));
    // The detail pane only appears on wide terminals.
    assert!(!screen.contains("COMMIT AAAAAAA"));
}

#[test]
fn renders_commit_detail_beside_the_list_on_wide_terminals() {
    let view = loaded_view();
    let screen = render(140, 40, &view, false);
    assert!(screen.contains("COMMIT AAAAAAA"));
    assert!(screen.contains("unathi@example.test"));
    assert!(screen.contains("author"));
    assert!(screen.contains("parents"));
    assert!(screen.contains("Talks to redis with a TTL."));
    assert!(screen.contains("Refs: #42"));
    assert!(screen.contains("FILES · 2 FILES  +12 −0"));
    assert!(screen.contains("+12 −0"));
    assert!(screen.contains("src/services/redis.py"));
    assert!(screen.contains("bin"));
    assert!(screen.contains("3h ago"));
}

#[test]
fn enter_swaps_the_list_for_the_detail_on_narrow_terminals() {
    let mut view = loaded_view();
    key(&mut view, KeyCode::Enter);
    let screen = render(80, 24, &view, true);
    assert!(screen.contains("COMMIT AAAAAAA"));
    assert!(!screen.contains("COMMITS ·"));
    assert!(screen.contains("esc List"));
    key(&mut view, KeyCode::Esc);
    assert!(!view.closed, "esc first closes the detail");
    assert!(render(80, 24, &view, true).contains("COMMITS ·"));
    key(&mut view, KeyCode::Esc);
    assert!(view.closed, "esc on the list returns to Changes");
}

#[test]
fn the_cursor_skips_rows_that_only_draw_lanes() {
    let mut view = loaded_view();
    key(&mut view, KeyCode::Down);
    assert_eq!(view.selected, 2, "skips the `|\\` connector");
    assert_eq!(
        view.request,
        Some(CommitsRequest::Detail(
            view.selected_commit().unwrap().hash.clone()
        ))
    );
    key(&mut view, KeyCode::Char('j'));
    key(&mut view, KeyCode::Char('j'));
    assert_eq!(view.selected, 5, "skips the `|/` connector");
    key(&mut view, KeyCode::Down);
    assert_eq!(view.selected, 5, "stops at the last commit");
    key(&mut view, KeyCode::Up);
    assert_eq!(view.selected, 3);
    key(&mut view, KeyCode::Home);
    assert_eq!(view.selected, 0);
    key(&mut view, KeyCode::Char('G'));
    assert_eq!(view.selected, 5);
    key(&mut view, KeyCode::Char('g'));
    assert_eq!(view.selected, 0);
}

#[test]
fn a_detail_for_a_commit_the_cursor_left_is_dropped() {
    let mut view = loaded_view();
    let first = view.selected_commit().unwrap().hash.clone();
    key(&mut view, KeyCode::Down);
    view.request = None;
    view.apply(CommitsEvent::Detail(first.clone(), Ok(detail(&first))));
    let screen = render(140, 40, &view, false);
    assert!(
        screen.contains("Reading commit…"),
        "the old commit's detail must not show under the new one"
    );
}

#[test]
fn toggling_scope_reloads_and_keeps_the_selected_commit() {
    let mut view = loaded_view();
    key(&mut view, KeyCode::Down);
    let kept = view.selected_commit().unwrap().hash.clone();
    view.request = None;
    key(&mut view, KeyCode::Char('a'));
    assert_eq!(
        view.request.take(),
        Some(CommitsRequest::Load {
            scope: LogScope::Head,
            limit: PAGE
        })
    );
    assert!(render(80, 24, &view, true).contains("a All branches"));
    // An answer for the scope we just left is ignored.
    view.apply(CommitsEvent::Loaded {
        scope: LogScope::All,
        limit: PAGE,
        result: Err("late".to_owned()),
    });
    assert!(view.load_error.is_none());
    view.apply(CommitsEvent::Loaded {
        scope: LogScope::Head,
        limit: PAGE,
        result: Ok(page()),
    });
    assert_eq!(view.selected_commit().unwrap().hash, kept);
    assert!(render(80, 24, &view, true).contains("COMMITS · CURRENT BRANCH"));
}

#[test]
fn reaching_the_end_of_a_full_page_fetches_older_commits() {
    let mut view = loaded_view();
    view.more = true;
    view.request = None;
    key(&mut view, KeyCode::End);
    assert_eq!(
        view.request,
        Some(CommitsRequest::Load {
            scope: LogScope::All,
            limit: PAGE * 2
        })
    );
    assert!(render(80, 24, &view, true).contains("reading history…"));
    // While that reload is pending, moving does not replace it with a detail request.
    key(&mut view, KeyCode::Up);
    assert!(matches!(view.request, Some(CommitsRequest::Load { .. })));
}

#[test]
fn shows_errors_and_the_empty_state_in_plain_words() {
    let mut view = CommitsView::open(PathBuf::from("C:\\projects\\lumina"), None);
    view.request = None;
    assert!(render(80, 24, &view, true).contains("Reading history…"));
    view.apply(CommitsEvent::Loaded {
        scope: LogScope::All,
        limit: PAGE,
        result: Err("Could not inspect C:\\x: not a repository".to_owned()),
    });
    let screen = render(100, 24, &view, true);
    assert!(screen.contains("Could not read the history."));
    assert!(screen.contains("Press r to retry."));

    view.apply(CommitsEvent::Loaded {
        scope: LogScope::All,
        limit: PAGE,
        result: Ok(LogPage {
            rows: Vec::new(),
            more: false,
        }),
    });
    assert!(render(100, 24, &view, true).contains("No commits yet."));
    key(&mut view, KeyCode::Down);
    key(&mut view, KeyCode::End);
    assert!(view.request.is_none(), "nothing to select or fetch");
}

#[test]
fn quit_keys_and_refresh() {
    let mut view = loaded_view();
    view.request = None;
    key(&mut view, KeyCode::Char('r'));
    assert!(matches!(view.request, Some(CommitsRequest::Load { .. })));
    key(&mut view, KeyCode::Char('q'));
    assert!(view.should_quit);
    let mut view = loaded_view();
    view.handle_key(KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert!(view.should_quit);
}

#[test]
fn works_without_color() {
    let screen = render(80, 24, &loaded_view(), false);
    assert!(
        screen.contains("[⎇ main]"),
        "refs are readable without colour"
    );
    assert!(screen.contains("> ●"));
}

#[test]
fn reads_a_real_repository_end_to_end() {
    let directory = TestDirectory::new("commits-screen");
    let repo = directory.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    run_git(&repo, &["init", "-b", "main"]);
    run_git(&repo, &["config", "user.name", "Forge Test"]);
    run_git(&repo, &["config", "user.email", "forge@example.test"]);
    run_git(&repo, &["config", "commit.gpgsign", "false"]);
    fs::write(repo.join("a.txt"), "1\n").unwrap();
    run_git(&repo, &["add", "."]);
    run_git(&repo, &["commit", "-m", "feat: first"]);
    run_git(&repo, &["switch", "-c", "side"]);
    fs::write(repo.join("b.txt"), "2\n").unwrap();
    run_git(&repo, &["add", "."]);
    run_git(&repo, &["commit", "-m", "feat: side"]);
    run_git(&repo, &["switch", "main"]);
    fs::write(repo.join("c.txt"), "3\n").unwrap();
    run_git(&repo, &["add", "."]);
    run_git(&repo, &["commit", "-m", "fix: main"]);
    run_git(&repo, &["merge", "--no-ff", "side", "-m", "Merge side"]);

    let mut view = CommitsView::open(repo.clone(), None);
    let Some(CommitsRequest::Load { scope, limit }) = view.request.take() else {
        panic!("opening asks for the history");
    };
    let result = read_log(&repo, scope, limit);
    view.apply(CommitsEvent::Loaded {
        scope,
        limit,
        result,
    });
    let Some(CommitsRequest::Detail(hash)) = view.request.take() else {
        panic!("loading asks for the detail");
    };
    assert_eq!(hash, git_text(&repo, &["rev-parse", "HEAD"]).trim());
    let result = read_commit_detail(&repo, &hash);
    view.apply(CommitsEvent::Detail(hash, result));

    let screen = render(140, 40, &view, true);
    assert!(screen.contains("Merge side"));
    assert!(screen.contains("feat: side"));
    assert!(screen.contains("fix: main"));
    assert!(screen.contains("[⎇ main]"));
    assert!(screen.contains("[side]"));
    assert!(screen.contains("BROUGHT IN · 1 FILE"));
    assert!(screen.contains("b.txt"));
}

#[test]
fn the_tab_strip_marks_commits_and_number_keys_switch_screens() {
    use crate::screens::Screen;
    let mut view = loaded_view();
    let screen = render(80, 24, &view, false);
    assert!(screen.contains("[2 Commits]"));
    assert!(screen.contains("1 Changes"));
    assert!(screen.contains("3 Branches"));
    key(&mut view, KeyCode::Char('2'));
    assert!(!view.closed && view.goto.is_none(), "already here");
    key(&mut view, KeyCode::Char('3'));
    assert_eq!(view.goto, Some(Screen::Branches));
    key(&mut view, KeyCode::Char('1'));
    assert!(view.closed, "1 goes back to Changes");
}
