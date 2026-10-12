use super::{BranchesEvent, BranchesRequest, BranchesView, Operation, draw_branches};
use crate::{
    git::{
        branches::{Branch, BranchKind, BranchList, SwitchTarget, Worktree},
        repository::BranchTracking,
    },
    test_support::{TestDirectory, git_text, run_git},
    theme::Theme,
    ui::unix_now,
};
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::{Terminal, backend::TestBackend};
use std::{fs, path::PathBuf};

fn branch(name: &str, kind: BranchKind) -> Branch {
    Branch {
        name: name.to_owned(),
        kind,
        current: false,
        upstream: None,
        ahead: 0,
        behind: 0,
        gone: false,
        short: "abc1234".to_owned(),
        hash: format!("abc1234{}", "0".repeat(33)),
        subject: format!("work on {name}"),
        time: unix_now() - 2 * 3600,
        merged: false,
        unique: None,
        worktree: None,
    }
}

/// main (current, ahead 2), a merged branch, an unmerged one with 3 commits, a branch
/// whose remote is gone, one held by another worktree, and one remote branch.
fn list() -> BranchList {
    let mut main = branch("main", BranchKind::Local);
    main.current = true;
    main.merged = true;
    main.unique = Some(0);
    main.upstream = Some("origin/main".to_owned());
    main.ahead = 2;
    let mut done = branch("chore/done", BranchKind::Local);
    done.merged = true;
    done.unique = Some(0);
    let mut wip = branch("feat/redis-cache", BranchKind::Local);
    wip.unique = Some(3);
    wip.upstream = Some("origin/feat/redis-cache".to_owned());
    let mut stale = branch("fix/stale", BranchKind::Local);
    stale.upstream = Some("origin/fix/stale".to_owned());
    stale.gone = true;
    stale.unique = Some(1);
    let mut held = branch("fix/linked", BranchKind::Local);
    held.worktree = Some("C:/work/lumina-fix".to_owned());
    let mut remote = branch("origin/feat/remote-only", BranchKind::Remote);
    remote.unique = Some(4);
    BranchList {
        branches: vec![main, done, wip, stale, held, remote],
        worktrees: vec![
            Worktree {
                path: "C:/projects/lumina".to_owned(),
                branch: Some("main".to_owned()),
                short: "abc1234".to_owned(),
                detached: false,
                bare: false,
                current: true,
            },
            Worktree {
                path: "C:/work/lumina-fix".to_owned(),
                branch: Some("fix/linked".to_owned()),
                short: "def5678".to_owned(),
                detached: false,
                bare: false,
                current: false,
            },
        ],
        detached: None,
        dirty_files: 0,
    }
}

fn tracking() -> BranchTracking {
    BranchTracking {
        branch: "main".to_owned(),
        upstream: true,
        ahead: 2,
        behind: 0,
        unborn: false,
    }
}

fn loaded_with(list: BranchList) -> BranchesView {
    let mut view = BranchesView::open(PathBuf::from("C:\\projects\\lumina"), Some(tracking()));
    assert_eq!(view.request.take(), Some(BranchesRequest::Load));
    view.apply(BranchesEvent::Loaded(Ok(list)));
    view
}

fn loaded_view() -> BranchesView {
    loaded_with(list())
}

fn render(width: u16, height: u16, view: &BranchesView, colors: bool) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
    terminal
        .draw(|frame| draw_branches(frame, view, &Theme::new(colors), Some("unathi")))
        .expect("branches screen renders");
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

fn key(view: &mut BranchesView, code: KeyCode) {
    view.handle_key(code, KeyModifiers::NONE);
}

fn type_text(view: &mut BranchesView, text: &str) {
    for character in text.chars() {
        key(view, KeyCode::Char(character));
    }
}

fn move_to(view: &mut BranchesView, name: &str) {
    view.selected = view
        .branches()
        .iter()
        .position(|branch| branch.name == name)
        .expect("branch is in the sample list");
}

fn notice(view: &BranchesView) -> String {
    view.notice
        .as_ref()
        .map(|n| n.text.clone())
        .unwrap_or_default()
}

#[test]
fn renders_sections_rows_and_keys_at_80x24() {
    let view = loaded_view();
    let screen = render(80, 24, &view, true);
    assert!(screen.contains("FORGE"));
    assert!(screen.contains("⎇ main ↑2 ↓0"));
    assert!(screen.contains("LOCAL · 5"));
    assert!(screen.contains("REMOTE · 1"));
    assert!(
        screen.contains("> ● main"),
        "the cursor is on the current branch"
    );
    assert!(screen.contains("↑2"));
    assert!(screen.contains("local"));
    assert!(screen.contains("gone"));
    assert!(screen.contains("worktree"));
    assert!(screen.contains("origin/feat/remote-only"));
    assert!(screen.contains("2h"));
    assert!(screen.contains("enter  Switch") || screen.contains(" enter  Switch"));
    assert!(screen.contains("n New"));
    assert!(screen.contains("R Rename"));
    assert!(
        !screen.contains("BRANCH · MAIN"),
        "detail needs a wider terminal"
    );
}

#[test]
fn rename_and_delete_hints_only_show_for_local_branches() {
    let mut view = loaded_view();
    assert!(render(100, 24, &view, true).contains("x Delete"));
    move_to(&mut view, "origin/feat/remote-only");
    let screen = render(100, 24, &view, true);
    assert!(!screen.contains("x Delete"));
    assert!(!screen.contains("R Rename"));
}

#[test]
fn renders_branch_detail_and_worktrees_on_wide_terminals() {
    let mut view = loaded_view();
    move_to(&mut view, "feat/redis-cache");
    let screen = render(140, 40, &view, false);
    assert!(screen.contains("BRANCH · FEAT/REDIS-CACHE"));
    assert!(screen.contains("work on feat/redis-cache"));
    assert!(screen.contains("origin/feat/redis-cache · ↑0 ↓0"));
    assert!(screen.contains("3 commits are not in main"));
    assert!(screen.contains("WORKTREES · 2"));
    assert!(screen.contains("C:/work/lumina-fix"));
    assert!(screen.contains("⎇ fix/linked"));

    move_to(&mut view, "chore/done");
    assert!(render(140, 40, &view, false).contains("every commit is already in main"));
    move_to(&mut view, "fix/stale");
    assert!(render(140, 40, &view, false).contains("was deleted on the remote"));
    move_to(&mut view, "fix/linked");
    assert!(render(140, 40, &view, false).contains("checked out in C:/work/lumina-fix"));
}

#[test]
fn tab_swaps_list_and_detail_on_narrow_terminals() {
    let mut view = loaded_view();
    key(&mut view, KeyCode::Tab);
    let screen = render(80, 24, &view, true);
    assert!(screen.contains("BRANCH · MAIN"));
    assert!(!screen.contains("LOCAL · 5"));
    assert!(screen.contains("esc List"));
    key(&mut view, KeyCode::Esc);
    assert!(!view.closed);
    assert!(render(80, 24, &view, true).contains("LOCAL · 5"));
    key(&mut view, KeyCode::Esc);
    assert!(view.closed);
}

#[test]
fn the_cursor_moves_and_stops_at_the_ends() {
    let mut view = loaded_view();
    key(&mut view, KeyCode::Up);
    assert_eq!(view.selected, 0);
    key(&mut view, KeyCode::Char('j'));
    key(&mut view, KeyCode::Down);
    assert_eq!(view.selected, 2);
    key(&mut view, KeyCode::End);
    assert_eq!(view.selected, 5);
    key(&mut view, KeyCode::Down);
    assert_eq!(view.selected, 5);
    key(&mut view, KeyCode::Char('g'));
    assert_eq!(view.selected, 0);
}

#[test]
fn enter_switches_when_the_tree_is_clean() {
    let mut view = loaded_view();
    move_to(&mut view, "chore/done");
    key(&mut view, KeyCode::Enter);
    assert_eq!(
        view.request,
        Some(BranchesRequest::Run(Operation::Switch {
            target: SwitchTarget {
                name: "chore/done".to_owned(),
                track: None
            },
            stash: false
        }))
    );
    assert!(render(80, 24, &view, true).contains("switching…"));
    // Further changes wait for this one.
    key(&mut view, KeyCode::Char('x'));
    assert_eq!(notice(&view), "Still working on the last change…");
}

#[test]
fn enter_on_the_current_branch_or_a_worktree_branch_explains_instead_of_running() {
    let mut view = loaded_view();
    key(&mut view, KeyCode::Enter);
    assert_eq!(notice(&view), "Already on main.");
    assert!(view.request.is_none());
    move_to(&mut view, "fix/linked");
    key(&mut view, KeyCode::Enter);
    assert!(notice(&view).contains("checked out in another worktree (C:/work/lumina-fix)"));
    assert!(view.request.is_none());
}

#[test]
fn a_remote_branch_is_checked_out_as_a_new_tracking_branch() {
    let mut view = loaded_view();
    move_to(&mut view, "origin/feat/remote-only");
    key(&mut view, KeyCode::Enter);
    assert_eq!(
        view.request,
        Some(BranchesRequest::Run(Operation::Switch {
            target: SwitchTarget {
                name: "feat/remote-only".to_owned(),
                track: Some("origin/feat/remote-only".to_owned())
            },
            stash: false
        }))
    );

    // When a local branch of that name exists, it is used instead of creating another.
    let mut with_local = list();
    with_local
        .branches
        .push(branch("feat/remote-only", BranchKind::Local));
    let mut view = loaded_with(with_local);
    move_to(&mut view, "origin/feat/remote-only");
    key(&mut view, KeyCode::Enter);
    let Some(BranchesRequest::Run(Operation::Switch { target, .. })) = &view.request else {
        panic!("switch requested");
    };
    assert_eq!(target.track, None);
}

#[test]
fn switching_with_uncommitted_changes_asks_what_to_do() {
    let mut dirty = list();
    dirty.dirty_files = 3;
    let mut view = loaded_with(dirty);
    move_to(&mut view, "chore/done");
    key(&mut view, KeyCode::Enter);
    assert!(
        view.request.is_none(),
        "nothing runs before the user chooses"
    );
    let screen = render(80, 24, &view, true);
    assert!(screen.contains("UNCOMMITTED CHANGES"));
    assert!(screen.contains("3 files have uncommitted changes."));
    assert!(screen.contains("Switching to chore/done:"));
    assert!(screen.contains("git stash pop"));
    assert!(screen.contains("s  Stash & switch") || screen.contains("s Stash & switch"));

    key(&mut view, KeyCode::Esc);
    assert!(view.request.is_none());
    assert!(!render(80, 24, &view, true).contains("UNCOMMITTED CHANGES"));

    key(&mut view, KeyCode::Enter);
    key(&mut view, KeyCode::Char('s'));
    let Some(BranchesRequest::Run(Operation::Switch { stash, .. })) = view.request.take() else {
        panic!("switch requested");
    };
    assert!(stash);

    let mut view = {
        let mut dirty = list();
        dirty.dirty_files = 1;
        loaded_with(dirty)
    };
    move_to(&mut view, "chore/done");
    key(&mut view, KeyCode::Enter);
    assert!(render(80, 24, &view, true).contains("1 file has uncommitted changes."));
    key(&mut view, KeyCode::Char('c'));
    let Some(BranchesRequest::Run(Operation::Switch { stash, .. })) = view.request.take() else {
        panic!("switch requested");
    };
    assert!(!stash, "carrying over leaves the changes in place");
}

#[test]
fn creating_a_branch_validates_the_name_inside_the_dialog() {
    let mut view = loaded_view();
    move_to(&mut view, "feat/redis-cache");
    key(&mut view, KeyCode::Char('n'));
    let screen = render(80, 24, &view, true);
    assert!(screen.contains("NEW BRANCH"));
    assert!(screen.contains("Starting from feat/redis-cache."));

    key(&mut view, KeyCode::Enter);
    assert!(render(80, 24, &view, true).contains("Type a branch name first."));
    type_text(&mut view, "bad name");
    key(&mut view, KeyCode::Enter);
    assert!(render(80, 24, &view, true).contains("is not a valid branch name"));
    assert!(view.request.is_none(), "the dialog stays open");

    for _ in 0.."bad name".len() {
        key(&mut view, KeyCode::Backspace);
    }
    type_text(&mut view, "main");
    key(&mut view, KeyCode::Enter);
    assert!(render(80, 24, &view, true).contains("A branch named main already exists."));

    for _ in 0.."main".len() {
        key(&mut view, KeyCode::Backspace);
    }
    type_text(&mut view, "feat/cache-ttl");
    key(&mut view, KeyCode::Enter);
    assert_eq!(
        view.request,
        Some(BranchesRequest::Run(Operation::Create {
            name: "feat/cache-ttl".to_owned(),
            start: "refs/heads/feat/redis-cache".to_owned()
        }))
    );
    assert!(!render(80, 24, &view, true).contains("NEW BRANCH"));
}

#[test]
fn escape_cancels_the_name_dialog_without_leaving_the_screen() {
    let mut view = loaded_view();
    key(&mut view, KeyCode::Char('n'));
    type_text(&mut view, "q");
    assert!(!view.should_quit, "letters are text while typing a name");
    key(&mut view, KeyCode::Esc);
    assert!(!view.closed);
    assert!(view.request.is_none());
    assert!(!render(80, 24, &view, true).contains("NEW BRANCH"));
}

#[test]
fn a_new_branch_can_start_from_a_remote_branch() {
    let mut view = loaded_view();
    move_to(&mut view, "origin/feat/remote-only");
    key(&mut view, KeyCode::Char('n'));
    type_text(&mut view, "mine");
    key(&mut view, KeyCode::Enter);
    let Some(BranchesRequest::Run(Operation::Create { start, .. })) = &view.request else {
        panic!("create requested");
    };
    assert_eq!(start, "refs/remotes/origin/feat/remote-only");
}

#[test]
fn renaming_starts_from_the_current_name() {
    let mut view = loaded_view();
    move_to(&mut view, "chore/done");
    key(&mut view, KeyCode::Char('R'));
    let screen = render(80, 24, &view, true);
    assert!(screen.contains("RENAME BRANCH"));
    assert!(screen.contains("New name for chore/done:"));
    assert!(screen.contains("▸ chore/done"));

    key(&mut view, KeyCode::Enter);
    assert!(render(80, 24, &view, true).contains("That is already its name."));
    type_text(&mut view, "-2");
    key(&mut view, KeyCode::Enter);
    assert_eq!(
        view.request,
        Some(BranchesRequest::Run(Operation::Rename {
            old: "chore/done".to_owned(),
            new: "chore/done-2".to_owned()
        }))
    );

    let mut view = loaded_view();
    move_to(&mut view, "origin/feat/remote-only");
    key(&mut view, KeyCode::Char('R'));
    assert!(notice(&view).contains("renamed on the remote"));
}

#[test]
fn deleting_a_merged_branch_is_safe_and_names_the_undo() {
    let mut view = loaded_view();
    move_to(&mut view, "chore/done");
    key(&mut view, KeyCode::Char('x'));
    let screen = render(80, 24, &view, true);
    assert!(screen.contains("DELETE BRANCH"));
    assert!(screen.contains("Delete branch chore/done?"));
    assert!(screen.contains("already in main, so nothing is lost"));
    assert!(screen.contains("git branch chore/done abc1234"));
    key(&mut view, KeyCode::Enter);
    assert!(view.request.is_none(), "enter does not confirm a delete");
    key(&mut view, KeyCode::Char('y'));
    assert_eq!(
        view.request,
        Some(BranchesRequest::Run(Operation::Delete {
            name: "chore/done".to_owned(),
            force: false
        }))
    );
}

#[test]
fn deleting_unmerged_work_warns_how_much_would_be_lost_and_forces_only_on_y() {
    let mut view = loaded_view();
    move_to(&mut view, "feat/redis-cache");
    key(&mut view, KeyCode::Char('x'));
    let screen = render(100, 24, &view, true);
    assert!(screen.contains("You will lose: 3 commits that are in no other branch (main)"));
    key(&mut view, KeyCode::Esc);
    assert!(view.request.is_none());
    key(&mut view, KeyCode::Char('x'));
    key(&mut view, KeyCode::Char('n'));
    assert!(view.request.is_none());
    key(&mut view, KeyCode::Char('x'));
    key(&mut view, KeyCode::Char('y'));
    assert_eq!(
        view.request,
        Some(BranchesRequest::Run(Operation::Delete {
            name: "feat/redis-cache".to_owned(),
            force: true
        }))
    );
}

#[test]
fn some_branches_cannot_be_deleted_and_say_why() {
    let mut view = loaded_view();
    key(&mut view, KeyCode::Char('x'));
    assert!(notice(&view).contains("You are on this branch"));
    move_to(&mut view, "fix/linked");
    key(&mut view, KeyCode::Char('x'));
    assert!(notice(&view).contains("another worktree"));
    move_to(&mut view, "origin/feat/remote-only");
    key(&mut view, KeyCode::Char('x'));
    assert!(notice(&view).contains("git push origin --delete feat/remote-only"));
    assert!(view.request.is_none());
}

#[test]
fn a_finished_operation_reloads_and_follows_the_branch() {
    let mut view = loaded_view();
    move_to(&mut view, "chore/done");
    key(&mut view, KeyCode::Enter);
    view.request = None;
    view.apply(BranchesEvent::Done(
        Ok("Switched to chore/done.".to_owned()),
    ));
    assert_eq!(notice(&view), "Switched to chore/done.");
    assert_eq!(view.request, Some(BranchesRequest::Load));

    // The new list has chore/done checked out; the cursor lands on it and the title follows.
    let mut after = list();
    after.branches[0].current = false;
    after.branches[1].current = true;
    view.apply(BranchesEvent::Loaded(Ok(after)));
    assert_eq!(view.selected_branch().unwrap().name, "chore/done");
    let screen = render(80, 24, &view, true);
    assert!(screen.contains("⎇ chore/done"));
    assert!(screen.contains("✓ Switched to chore/done."));
}

#[test]
fn a_failed_operation_reports_and_keeps_the_cursor() {
    let mut view = loaded_view();
    move_to(&mut view, "chore/done");
    key(&mut view, KeyCode::Enter);
    view.request = None;
    view.apply(BranchesEvent::Done(Err(
        "Could not switch to chore/done. error: Your local changes would be overwritten. Press r to refresh, then try again."
            .to_owned(),
    )));
    assert!(view.notice.as_ref().unwrap().error);
    view.apply(BranchesEvent::Loaded(Ok(list())));
    assert_eq!(view.selected_branch().unwrap().name, "chore/done");
    assert!(render(100, 24, &view, true).contains("✕ Could not switch to chore/done."));
}

#[test]
fn a_reload_keeps_the_cursor_on_the_same_branch_by_name() {
    let mut view = loaded_view();
    move_to(&mut view, "fix/stale");
    key(&mut view, KeyCode::Char('r'));
    assert_eq!(view.request.take(), Some(BranchesRequest::Load));
    let mut shorter = list();
    shorter.branches.remove(1);
    view.apply(BranchesEvent::Loaded(Ok(shorter)));
    assert_eq!(view.selected_branch().unwrap().name, "fix/stale");
    // A deleted branch leaves the cursor in range.
    view.selected = 4;
    view.select_after = Some("gone".to_owned());
    let mut tiny = list();
    tiny.branches.truncate(2);
    view.apply(BranchesEvent::Loaded(Ok(tiny)));
    assert_eq!(view.selected, 1);
}

#[test]
fn shows_loading_errors_and_the_empty_state_in_plain_words() {
    let mut view = BranchesView::open(PathBuf::from("C:\\projects\\lumina"), None);
    view.request = None;
    assert!(render(80, 24, &view, true).contains("Reading branches…"));
    view.apply(BranchesEvent::Loaded(Err(
        "Could not inspect C:\\x.".to_owned()
    )));
    let screen = render(100, 24, &view, true);
    assert!(screen.contains("Could not read the branches."));
    assert!(screen.contains("Press r to retry."));
    view.apply(BranchesEvent::Loaded(Ok(BranchList {
        branches: Vec::new(),
        worktrees: Vec::new(),
        detached: None,
        dirty_files: 0,
    })));
    assert!(render(100, 24, &view, true).contains("No branches yet."));
    key(&mut view, KeyCode::Enter);
    key(&mut view, KeyCode::Char('x'));
    key(&mut view, KeyCode::Char('R'));
    key(&mut view, KeyCode::Down);
    assert!(view.request.is_none(), "nothing to act on");
    key(&mut view, KeyCode::Char('n'));
    assert!(notice(&view).contains("Make a commit in Changes first"));
}

#[test]
fn a_detached_head_is_called_out_in_the_local_heading() {
    let mut detached = list();
    for branch in &mut detached.branches {
        branch.current = false;
    }
    detached.detached = Some("9f8e7d6".to_owned());
    let view = loaded_with(detached);
    assert!(render(100, 24, &view, true).contains("LOCAL · 5 · HEAD DETACHED AT 9F8E7D6"));
    assert_eq!(view.tracking.as_ref().unwrap().branch, "");
}

#[test]
fn quit_keys_and_works_without_color() {
    let mut view = loaded_view();
    key(&mut view, KeyCode::Char('q'));
    assert!(view.should_quit);
    let mut view = loaded_view();
    view.handle_key(KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert!(view.should_quit);
    let screen = render(80, 24, &loaded_view(), false);
    assert!(screen.contains("> ● main"));
    assert!(screen.contains("worktree"));
}

/// Applies the screen's pending request the way the run loop does.
fn settle(view: &mut BranchesView, repo: &std::path::Path) {
    while let Some(request) = view.request.take() {
        match request {
            BranchesRequest::Load => {
                let result = crate::git::branches::read_branches(repo);
                view.apply(BranchesEvent::Loaded(result));
            }
            BranchesRequest::Run(operation) => {
                let result = operation.run(repo);
                view.apply(BranchesEvent::Done(result));
            }
        }
    }
}

#[test]
fn creates_switches_renames_and_deletes_in_a_real_repository() {
    let directory = TestDirectory::new("branches-screen");
    let repo = directory.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    run_git(&repo, &["init", "-b", "main"]);
    run_git(&repo, &["config", "user.name", "Forge Test"]);
    run_git(&repo, &["config", "user.email", "forge@example.test"]);
    run_git(&repo, &["config", "commit.gpgsign", "false"]);
    fs::write(repo.join("a.txt"), "1\n").unwrap();
    run_git(&repo, &["add", "."]);
    run_git(&repo, &["commit", "-m", "feat: first"]);

    let mut view = BranchesView::open(repo.clone(), None);
    settle(&mut view, &repo);
    assert_eq!(view.branches().len(), 1);

    // n, type a name, enter: the branch exists and is checked out.
    key(&mut view, KeyCode::Char('n'));
    type_text(&mut view, "feat/screen");
    key(&mut view, KeyCode::Enter);
    settle(&mut view, &repo);
    assert_eq!(
        git_text(&repo, &["branch", "--show-current"]).trim(),
        "feat/screen"
    );
    assert_eq!(view.selected_branch().unwrap().name, "feat/screen");
    assert!(notice(&view).starts_with("Created feat/screen"));
    assert_eq!(view.tracking.as_ref().unwrap().branch, "feat/screen");

    // Rename it.
    key(&mut view, KeyCode::Char('R'));
    for _ in 0.."feat/screen".len() {
        key(&mut view, KeyCode::Backspace);
    }
    type_text(&mut view, "feat/renamed");
    key(&mut view, KeyCode::Enter);
    settle(&mut view, &repo);
    assert_eq!(
        git_text(&repo, &["branch", "--show-current"]).trim(),
        "feat/renamed"
    );

    // Dirty tree: a tracked edit makes the switch ask, and stash-and-switch keeps the work.
    move_to(&mut view, "main");
    fs::write(repo.join("a.txt"), "edited\n").unwrap();
    key(&mut view, KeyCode::Char('r'));
    settle(&mut view, &repo);
    move_to(&mut view, "main");
    key(&mut view, KeyCode::Enter);
    assert!(render(80, 24, &view, true).contains("1 file has uncommitted changes."));
    key(&mut view, KeyCode::Char('s'));
    settle(&mut view, &repo);
    assert_eq!(
        git_text(&repo, &["branch", "--show-current"]).trim(),
        "main"
    );
    assert!(notice(&view).contains("git stash pop"));
    assert_eq!(view.list.as_ref().unwrap().dirty_files, 0);
    assert!(git_text(&repo, &["stash", "list"]).contains("forge: before switching to main"));

    // Delete the merged branch; the notice carries the undo command.
    move_to(&mut view, "feat/renamed");
    key(&mut view, KeyCode::Char('x'));
    key(&mut view, KeyCode::Char('y'));
    settle(&mut view, &repo);
    assert!(notice(&view).contains("Undo with: git branch feat/renamed "));
    assert!(view.branches().iter().all(|b| b.name != "feat/renamed"));
}

#[test]
fn the_tab_strip_marks_branches_and_number_keys_switch_screens() {
    use crate::screens::Screen;
    let mut view = loaded_view();
    let screen = render(80, 24, &view, false);
    assert!(screen.contains("[3 Branches]"));
    assert!(screen.contains("2 Commits"));
    key(&mut view, KeyCode::Char('3'));
    assert!(!view.closed && view.goto.is_none(), "already here");
    key(&mut view, KeyCode::Char('2'));
    assert_eq!(view.goto, Some(Screen::Commits));
    key(&mut view, KeyCode::Char('1'));
    assert!(view.closed, "1 goes back to Changes");
}

#[test]
fn digits_are_text_while_naming_a_branch() {
    let mut view = loaded_view();
    key(&mut view, KeyCode::Char('n'));
    type_text(&mut view, "fix-123");
    assert!(!view.closed && view.goto.is_none());
    key(&mut view, KeyCode::Enter);
    let Some(BranchesRequest::Run(Operation::Create { name, .. })) = &view.request else {
        panic!("create requested");
    };
    assert_eq!(name, "fix-123");
}

#[test]
fn slash_asks_for_the_palette_but_is_plain_text_while_naming_a_branch() {
    let mut view = loaded_view();
    key(&mut view, KeyCode::Char('/'));
    assert!(view.open_palette);
    view.open_palette = false;

    key(&mut view, KeyCode::Char('n'));
    type_text(&mut view, "feat/palette");
    assert!(!view.open_palette, "branch names contain slashes");
    key(&mut view, KeyCode::Enter);
    let Some(BranchesRequest::Run(Operation::Create { name, .. })) = &view.request else {
        panic!("create requested");
    };
    assert_eq!(name, "feat/palette");
}
