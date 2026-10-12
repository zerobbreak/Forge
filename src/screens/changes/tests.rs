use super::{ChangesEvent, ChangesRequest, ChangesView, EntryKey, Operation, Pane, draw_changes};
use crate::screens::Screen;
use crate::{
    git::{
        changes::{FileChange, FileState, WorkingTree, parse_diff, read_working_tree},
        repository::BranchTracking,
    },
    test_support::{TestDirectory, git_text, run_git},
    theme::Theme,
};
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::{Terminal, backend::TestBackend};
use std::{fs, path::PathBuf, sync::mpsc, time::Duration};

const TWO_HUNKS: &[u8] = b"diff --git a/src/services/cache.py b/src/services/cache.py\nindex 1..2 100644\n--- a/src/services/cache.py\n+++ b/src/services/cache.py\n@@ -10,4 +10,4 @@ class NoteCache:\n import asyncio\n-_cache = {}\n+TTL = 60 * 5\n from .redis import redis\n@@ -41,2 +41,3 @@ async def invalidate(id):\n     await redis.delete(id)\n+    log.debug(\"cache.invalidate\", id=id)\n";

fn change(path: &str, state: FileState, staged: bool, counts: (usize, usize)) -> FileChange {
    FileChange {
        path: path.to_owned(),
        original: None,
        state,
        staged,
        added: Some(counts.0),
        removed: Some(counts.1),
    }
}

fn tree() -> WorkingTree {
    WorkingTree {
        tracking: BranchTracking {
            branch: "feat/redis-cache".to_owned(),
            upstream: true,
            ahead: 2,
            behind: 0,
            unborn: false,
        },
        staged: vec![change(
            "src/services/redis.py",
            FileState::Added,
            true,
            (12, 0),
        )],
        unstaged: vec![
            change("src/services/cache.py", FileState::Modified, false, (2, 1)),
            change("src/legacy_cache.py", FileState::Deleted, false, (0, 48)),
            change("notes.txt", FileState::Untracked, false, (3, 0)),
        ],
        branches: 5,
        last_fetch: None,
    }
}

/// A view with the sample tree loaded and the cursor on the first unstaged file.
fn loaded_view() -> ChangesView {
    let mut view = ChangesView::open(PathBuf::from("C:\\projects\\lumina"));
    assert_eq!(view.request.take(), Some(ChangesRequest::Load));
    view.apply(ChangesEvent::Loaded(Ok(tree())));
    view.request = None;
    view.handle_key(KeyCode::Down, KeyModifiers::NONE);
    let Some(ChangesRequest::Diff(file)) = view.request.take() else {
        panic!("moving the cursor requests the file's diff");
    };
    assert_eq!(file.path, "src/services/cache.py");
    view.apply(ChangesEvent::Diff(
        EntryKey::of(&file),
        Ok(parse_diff(TWO_HUNKS)),
    ));
    view
}

fn render(width: u16, height: u16, view: &ChangesView, colors: bool) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
    terminal
        .draw(|frame| draw_changes(frame, view, &Theme::new(colors), Some("unathi")))
        .expect("changes screen renders");
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

fn key(view: &mut ChangesView, code: KeyCode) {
    view.handle_key(code, KeyModifiers::NONE);
}

#[test]
fn renders_lists_diff_and_keys_at_80x24() {
    let view = loaded_view();
    let screen = render(80, 24, &view, true);
    assert!(screen.contains("FORGE"));
    assert!(screen.contains("⎇ feat/redis-cache ↑2 ↓0"));
    assert!(screen.contains("@unathi"));
    assert!(screen.contains("STAGED · 1"));
    assert!(screen.contains("UNSTAGED · 3"));
    assert!(screen.contains("> M src/services/cache.py +2 −1"));
    assert!(screen.contains("−48"));
    assert!(screen.contains("DIFF · HUNK 1 OF 2"));
    assert!(screen.contains("▌@@ -10,4 +10,4 @@ class NoteCache:"));
    assert!(screen.contains("  11 + TTL = 60 * 5"));
    assert!(screen.contains("s  Stage hunk"));
    // The navigation column only appears on wide terminals.
    assert!(!screen.contains("REPOSITORY"));
}

#[test]
fn renders_navigation_and_summary_on_wide_terminals_without_color() {
    let view = loaded_view();
    let screen = render(140, 40, &view, false);
    assert!(screen.contains("REPOSITORY"));
    assert!(screen.contains("> 1 Changes"));
    assert!(screen.contains("2 Commits"));
    assert!(screen.contains("3 Branches"));
    assert!(screen.contains("Branches"));
    assert!(screen.contains("SUMMARY"));
    assert!(screen.contains("4 files · +17 −49"));
    assert!(screen.contains("never fetched"));
    assert!(
        screen.contains("▸ STAGED · 1"),
        "focus is visible without color"
    );
    assert!(screen.contains("esc Projects"));
}

#[test]
fn narrow_terminals_show_only_the_focused_pane() {
    let mut view = loaded_view();
    assert!(render(50, 20, &view, true).contains("UNSTAGED · 3"));
    key(&mut view, KeyCode::Tab);
    let screen = render(50, 20, &view, true);
    assert!(!screen.contains("UNSTAGED · 3"));
    assert!(screen.contains("▸ DIFF · HUNK 1 OF 2"));
}

#[test]
fn shows_clean_tree_and_load_errors() {
    let mut view = ChangesView::open(PathBuf::from("C:\\projects\\lumina"));
    assert!(render(80, 24, &view, true).contains("Reading working tree…"));
    view.apply(ChangesEvent::Loaded(Err("not a git repository".to_owned())));
    assert!(render(140, 24, &view, true).contains("Could not read this repository."));
    view.apply(ChangesEvent::Loaded(Ok(WorkingTree::default())));
    let screen = render(80, 24, &view, true);
    assert!(screen.contains("Working tree clean."));
    assert!(screen.contains("Select a file to see its changes."));
}

#[test]
fn diff_pane_moves_between_hunks_and_stages_the_selected_one() {
    let mut view = loaded_view();
    key(&mut view, KeyCode::Tab);
    assert_eq!(view.focus, Pane::Diff);
    key(&mut view, KeyCode::Down);
    assert_eq!(view.hunk, 1);
    key(&mut view, KeyCode::Down);
    assert_eq!(view.hunk, 1, "stays on the last hunk");
    assert!(render(80, 24, &view, true).contains("DIFF · HUNK 2 OF 2"));

    key(&mut view, KeyCode::Char('s'));
    let Some(ChangesRequest::Run(Operation::StageHunk { change, hunk })) = view.request.take()
    else {
        panic!("s stages the selected hunk");
    };
    assert_eq!(change.path, "src/services/cache.py");
    assert_eq!(hunk.index, 1);
    let patch = String::from_utf8(hunk.patch).unwrap();
    assert!(patch.contains("+    log.debug"));
    assert!(!patch.contains("+TTL"));
    assert!(render(80, 24, &view, true).contains("◐ staging…"));

    // Further changes wait for the running one.
    key(&mut view, KeyCode::Char('a'));
    assert_eq!(view.request, None);
}

#[test]
fn file_keys_stage_and_explain_misuse() {
    let mut view = loaded_view();
    key(&mut view, KeyCode::Char('a'));
    assert!(matches!(
        view.request.take(),
        Some(ChangesRequest::Run(Operation::StageFile(change))) if change.path == "src/services/cache.py"
    ));
    view.apply(ChangesEvent::Done(Ok(
        "Staged src/services/cache.py.".to_owned()
    )));
    assert_eq!(view.request.take(), Some(ChangesRequest::Load));
    assert!(render(80, 24, &view, true).contains("✓ Staged src/services/cache.py."));

    key(&mut view, KeyCode::Char('u'));
    assert!(render(80, 24, &view, true).contains("Not staged yet."));

    key(&mut view, KeyCode::Up);
    view.request = None;
    key(&mut view, KeyCode::Char('s'));
    assert_eq!(view.request, None);
    assert!(render(80, 24, &view, true).contains("Already staged."));
    key(&mut view, KeyCode::Char('u'));
    assert!(matches!(
        view.request.take(),
        Some(ChangesRequest::Run(Operation::UnstageFile {
            unborn: false,
            ..
        }))
    ));
}

#[test]
fn errors_are_shown_with_a_next_step() {
    let mut view = loaded_view();
    key(&mut view, KeyCode::Char('a'));
    view.apply(ChangesEvent::Done(Err(
        "Could not stage x. index.lock exists. Press r to refresh, then try again.".to_owned(),
    )));
    let screen = render(120, 30, &view, true);
    assert!(screen.contains("✕ Could not stage x. index.lock exists. Press r to refresh"));
}

#[test]
fn reload_keeps_the_cursor_and_walks_down_after_a_file_is_staged() {
    let mut view = loaded_view();
    let mut after = tree();
    let staged = after.unstaged.remove(0);
    after.staged.push(FileChange {
        staged: true,
        ..staged
    });
    view.apply(ChangesEvent::Loaded(Ok(after)));
    let selected = view.selected_change().expect("a file stays selected");
    assert_eq!(selected.path, "src/legacy_cache.py");
    assert!(!selected.staged);
}

#[test]
fn stale_diffs_for_another_file_are_ignored() {
    let mut view = loaded_view();
    view.apply(ChangesEvent::Diff(
        EntryKey {
            path: "notes.txt".to_owned(),
            staged: false,
        },
        Ok(parse_diff(b"")),
    ));
    assert_eq!(view.selected_diff().map(|diff| diff.hunks.len()), Some(2));
}

#[test]
fn discard_requires_explicit_confirmation() {
    let mut view = loaded_view();
    key(&mut view, KeyCode::Tab);
    key(&mut view, KeyCode::Char('x'));
    let screen = render(80, 24, &view, true);
    assert!(screen.contains("DISCARD CHANGES"));
    assert!(screen.contains("Discard hunk 1 of 2 in src/services/cache.py?"));
    assert!(screen.contains("+1 −1"));
    assert!(screen.contains("refs/forge/snapshots/"));

    // Enter and other keys never confirm; Esc cancels without running anything.
    key(&mut view, KeyCode::Enter);
    key(&mut view, KeyCode::Char('x'));
    assert_eq!(view.request, None);
    key(&mut view, KeyCode::Esc);
    assert!(view.confirm.is_none());
    assert_eq!(view.request, None);
    assert!(!view.closed, "esc only closed the dialog");

    key(&mut view, KeyCode::Char('x'));
    key(&mut view, KeyCode::Char('y'));
    let Some(ChangesRequest::Run(Operation::Discard(discard))) = view.request.take() else {
        panic!("y confirms the discard");
    };
    assert_eq!(discard.hunk.map(|hunk| hunk.index), Some(0));
}

#[test]
fn discard_from_the_file_list_covers_the_whole_file_and_skips_staged_entries() {
    let mut view = loaded_view();
    key(&mut view, KeyCode::Down);
    key(&mut view, KeyCode::Down);
    view.request = None;
    key(&mut view, KeyCode::Char('x'));
    let screen = render(80, 24, &view, true);
    assert!(screen.contains("Delete untracked file notes.txt?"));
    key(&mut view, KeyCode::Esc);

    key(&mut view, KeyCode::Up);
    key(&mut view, KeyCode::Up);
    key(&mut view, KeyCode::Up);
    key(&mut view, KeyCode::Char('x'));
    assert!(view.confirm.is_none());
    assert!(render(80, 24, &view, true).contains("Press u to unstage first."));
}

#[test]
fn escape_returns_to_projects_and_q_quits() {
    let mut view = loaded_view();
    key(&mut view, KeyCode::Esc);
    assert!(view.closed);
    let mut view = loaded_view();
    key(&mut view, KeyCode::Char('q'));
    assert!(view.should_quit);
}

/// Drives the real background requests against a temporary repository.
#[test]
fn stages_a_hunk_end_to_end_through_background_requests() {
    let directory = TestDirectory::new("changes-screen");
    let repo = directory.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    run_git(&repo, &["init", "-q"]);
    run_git(&repo, &["config", "user.name", "Forge Test"]);
    run_git(&repo, &["config", "user.email", "forge@example.test"]);
    run_git(&repo, &["config", "core.autocrlf", "false"]);
    let original: String = (1..=10).map(|n| format!("line {n}\n")).collect();
    fs::write(repo.join("file.txt"), &original).unwrap();
    run_git(&repo, &["add", "file.txt"]);
    run_git(&repo, &["commit", "-q", "-m", "base"]);
    fs::write(
        repo.join("file.txt"),
        original
            .replace("line 1\n", "line one\n")
            .replace("line 10\n", "line ten\n"),
    )
    .unwrap();

    let (sender, receiver) = mpsc::channel();
    let mut view = ChangesView::open(repo.clone());
    let pump = |view: &mut ChangesView| {
        while let Some(request) = view.request.take() {
            super::spawn_request(view.path.clone(), request, sender.clone());
            let (path, event) = receiver
                .recv_timeout(Duration::from_secs(20))
                .expect("background git finishes");
            assert_eq!(path, view.path);
            view.apply(event);
        }
    };
    pump(&mut view);
    assert_eq!(view.selected_diff().map(|diff| diff.hunks.len()), Some(2));

    key(&mut view, KeyCode::Char('s'));
    pump(&mut view);
    let cached = git_text(&repo, &["diff", "--cached"]);
    assert!(cached.contains("+line one"));
    assert!(!cached.contains("+line ten"));
    let tree = read_working_tree(&repo).unwrap();
    assert_eq!((tree.staged.len(), tree.unstaged.len()), (1, 1));
    assert!(render(80, 24, &view, true).contains("Staged hunk 1 of 2 in file.txt."));
}

#[test]
fn c_opens_the_commit_screen_with_staged_files() {
    let mut view = loaded_view();
    assert!(render(80, 24, &view, true).contains("c Commit"));
    key(&mut view, KeyCode::Char('c'));
    assert!(view.open_commit);
    let (staged, tracking) = view.commit_context();
    assert_eq!(staged.len(), 1);
    assert_eq!(
        tracking.map(|tracking| tracking.branch).as_deref(),
        Some("feat/redis-cache")
    );

    view.commit_finished(crate::ui::Notice::info("Committed abc1234 x."));
    assert_eq!(view.request, Some(ChangesRequest::Load));
    assert!(render(80, 24, &view, true).contains("✓ Committed abc1234 x."));
}

#[test]
fn the_tab_strip_is_visible_at_every_width() {
    let view = loaded_view();
    for (width, height) in [(80, 24), (50, 20), (140, 40)] {
        let screen = render(width, height, &view, false);
        assert!(screen.contains("[1 Changes]"), "{width}x{height}");
        assert!(screen.contains("2 Commits"), "{width}x{height}");
        assert!(screen.contains("3 Branches"), "{width}x{height}");
    }
}

#[test]
fn number_keys_open_the_other_screens_from_any_pane() {
    let mut view = loaded_view();
    key(&mut view, KeyCode::Char('2'));
    assert_eq!(view.open_screen.take(), Some(Screen::Commits));
    key(&mut view, KeyCode::Tab);
    key(&mut view, KeyCode::Char('3'));
    assert_eq!(view.open_screen.take(), Some(Screen::Branches));
    key(&mut view, KeyCode::Char('4'));
    assert_eq!(view.open_screen.take(), Some(Screen::PullRequests));
    key(&mut view, KeyCode::Char('p'));
    assert_eq!(view.open_screen.take(), Some(Screen::PullRequests));
    key(&mut view, KeyCode::Char('5'));
    assert!(
        view.notice
            .as_ref()
            .unwrap()
            .text
            .contains("Remote is not built yet")
    );
    assert_eq!(view.open_screen, None);
    key(&mut view, KeyCode::Char('1'));
    assert_eq!(view.focus, Pane::Files, "1 returns to the file list");
}

#[test]
fn the_sidebar_can_be_focused_and_its_labels_open_screens() {
    let mut view = loaded_view();
    // Before a wide render the sidebar is not on screen, so it cannot take focus.
    render(80, 24, &view, false);
    key(&mut view, KeyCode::Char('h'));
    assert_eq!(view.focus, Pane::Files);
    assert!(
        view.notice
            .as_ref()
            .unwrap()
            .text
            .contains("needs a wider window")
    );

    render(140, 40, &view, false);
    key(&mut view, KeyCode::Left);
    assert_eq!(view.focus, Pane::Nav);
    let screen = render(140, 40, &view, false);
    assert!(
        screen.contains("▸ REPOSITORY"),
        "focus is visible without colour"
    );
    assert!(screen.contains("> 1 Changes"));

    key(&mut view, KeyCode::Char('j'));
    assert!(render(140, 40, &view, false).contains("> 2 Commits"));
    key(&mut view, KeyCode::Down);
    assert!(render(140, 40, &view, false).contains("> 3 Branches"));
    key(&mut view, KeyCode::Down);
    key(&mut view, KeyCode::Down);
    key(&mut view, KeyCode::Down);
    assert!(
        render(140, 40, &view, false).contains("> 5 Remote"),
        "stops at the last entry"
    );
    key(&mut view, KeyCode::Up);
    key(&mut view, KeyCode::Enter);
    assert_eq!(
        view.open_screen.take(),
        Some(Screen::PullRequests),
        "enter on Pull Requests"
    );
    key(&mut view, KeyCode::Up);
    key(&mut view, KeyCode::Enter);
    assert_eq!(
        view.open_screen.take(),
        Some(Screen::Branches),
        "enter on Branches"
    );
    key(&mut view, KeyCode::Up);
    key(&mut view, KeyCode::Enter);
    assert_eq!(
        view.open_screen.take(),
        Some(Screen::Commits),
        "enter on Commits"
    );
    key(&mut view, KeyCode::Down);
    key(&mut view, KeyCode::Down);
    key(&mut view, KeyCode::Down);
    key(&mut view, KeyCode::Enter);
    assert!(
        view.notice
            .as_ref()
            .unwrap()
            .text
            .contains("Remote is not built yet")
    );
    assert_eq!(view.open_screen, None);

    key(&mut view, KeyCode::Esc);
    assert_eq!(view.focus, Pane::Files);
    assert!(
        !view.closed,
        "esc leaves the sidebar before leaving the screen"
    );
}

#[test]
fn shrinking_the_window_hands_focus_back_from_a_hidden_sidebar() {
    let mut view = loaded_view();
    render(140, 40, &view, false);
    key(&mut view, KeyCode::Char('h'));
    assert_eq!(view.focus, Pane::Nav);
    render(80, 24, &view, false);
    key(&mut view, KeyCode::Char('j'));
    assert_eq!(view.focus, Pane::Files);
}

#[test]
fn slash_asks_for_the_palette_even_with_the_sidebar_focused() {
    let mut view = loaded_view();
    key(&mut view, KeyCode::Char('/'));
    assert!(view.open_palette);
    view.open_palette = false;
    render(140, 40, &view, false);
    key(&mut view, KeyCode::Char('h'));
    assert_eq!(view.focus, Pane::Nav);
    key(&mut view, KeyCode::Char('/'));
    assert!(view.open_palette);
}
