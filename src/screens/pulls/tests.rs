use super::{PullsEvent, PullsRequest, PullsView, draw_pulls};
use crate::{
    git::repository::BranchTracking,
    github::pulls::{
        Check, CheckState, Decision, Mergeable, PullDetail, PullFile, PullFilter, PullRequest,
        PullState, Review, ReviewState,
    },
    screens::Screen,
    theme::Theme,
    ui::unix_now,
};
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::{Terminal, backend::TestBackend};
use std::path::PathBuf;

fn check(name: &str, state: CheckState) -> Check {
    Check {
        name: name.to_owned(),
        workflow: Some("CI".to_owned()),
        state,
    }
}

fn pull(number: u64, title: &str) -> PullRequest {
    PullRequest {
        number,
        title: title.to_owned(),
        author: "BagToad".to_owned(),
        state: PullState::Open,
        draft: false,
        head: format!("feat/pr-{number}"),
        base: "main".to_owned(),
        updated: unix_now() - 3 * 3600,
        decision: Decision::None,
        checks: Vec::new(),
        additions: 40,
        deletions: 8,
        changed_files: 3,
        labels: Vec::new(),
    }
}

/// An open PR with a failing check, a draft, an approved one with all checks green,
/// and a merged one.
fn pulls() -> Vec<PullRequest> {
    let mut failing = pull(14640, "Address review feedback");
    failing.decision = Decision::ReviewRequired;
    failing.labels = vec!["external".to_owned()];
    failing.checks = vec![
        check("build", CheckState::Passed),
        check("lint", CheckState::Passed),
        check("test", CheckState::Failed),
        check("deploy", CheckState::Pending),
        check("label", CheckState::Skipped),
    ];
    let mut draft = pull(14641, "WIP: new cache layer");
    draft.draft = true;
    let mut green = pull(14642, "Bump go-colorable");
    green.decision = Decision::Approved;
    green.checks = vec![
        check("build", CheckState::Passed),
        check("test", CheckState::Passed),
        check("lint", CheckState::Passed),
    ];
    let mut merged = pull(14600, "Release 2.0");
    merged.state = PullState::Merged;
    merged.decision = Decision::ChangesRequested;
    vec![failing, draft, green, merged]
}

fn detail(number: u64) -> PullDetail {
    PullDetail {
        number,
        body: "Fixes the cache.\nSecond line.".to_owned(),
        reviews: vec![
            Review {
                author: "williammartin".to_owned(),
                state: ReviewState::Approved,
                submitted: unix_now() - 86_400,
            },
            Review {
                author: "hubot".to_owned(),
                state: ReviewState::ChangesRequested,
                submitted: unix_now() - 7_200,
            },
        ],
        requested: vec!["tidy-dev".to_owned()],
        files: vec![
            PullFile {
                path: "src/cache.rs".to_owned(),
                additions: 30,
                deletions: 2,
            },
            PullFile {
                path: "README.md".to_owned(),
                additions: 10,
                deletions: 6,
            },
        ],
        commits: 3,
        comments: 1,
        mergeable: Mergeable::Conflicting,
    }
}

fn tracking() -> BranchTracking {
    BranchTracking {
        branch: "main".to_owned(),
        upstream: true,
        ahead: 0,
        behind: 0,
        unborn: false,
    }
}

/// A view with the sample list loaded; the first pull request's detail was requested.
fn loaded_view() -> PullsView {
    let mut view = PullsView::open(PathBuf::from("C:\\projects\\lumina"), Some(tracking()));
    assert_eq!(
        view.request.take(),
        Some(PullsRequest::List(PullFilter::Open))
    );
    view.apply(PullsEvent::Listed(PullFilter::Open, Ok(pulls())));
    view
}

fn render(width: u16, height: u16, view: &PullsView, colors: bool) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
    terminal
        .draw(|frame| draw_pulls(frame, view, &Theme::new(colors), Some("unathi")))
        .expect("pull requests screen renders");
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

fn key(view: &mut PullsView, code: KeyCode) {
    view.handle_key(code, KeyModifiers::NONE);
}

fn notice(view: &PullsView) -> String {
    view.notice
        .as_ref()
        .map(|n| n.text.clone())
        .unwrap_or_default()
}

#[test]
fn renders_the_list_with_remote_label_checks_and_review_state_at_80x24() {
    let view = loaded_view();
    let screen = render(80, 24, &view, true);
    assert!(screen.contains("[4 Pull Requests]"));
    assert!(screen.contains("PULL REQUESTS · OPEN · 4 · FROM GITHUB JUST NOW"));
    assert!(screen.contains("> #14640 ● Address review feedback"));
    assert!(
        screen.contains("✕ 1/4"),
        "a failing check wins: 1 of 4 counted checks"
    );
    assert!(screen.contains("review"));
    assert!(
        screen.contains("○ WIP: new cache layer"),
        "drafts are hollow"
    );
    assert!(screen.contains("✓ 3/3"));
    assert!(screen.contains("approved"));
    assert!(screen.contains("◆ Release 2.0"));
    assert!(screen.contains("changes"));
    assert!(screen.contains("3h"));
    assert!(screen.contains("enter  Open on GitHub") || screen.contains("Open on GitHub"));
    assert!(screen.contains("c Check out"));
    assert!(screen.contains("f open"));
    assert!(
        !screen.contains("PULL REQUEST #14640"),
        "detail needs a wider terminal"
    );
}

#[test]
fn the_author_column_appears_when_there_is_room() {
    let view = loaded_view();
    assert!(!render(70, 24, &view, true).contains("BagToad"));
    assert!(render(90, 24, &view, true).contains("BagToad"));
}

#[test]
fn renders_the_detail_beside_the_list_on_wide_terminals() {
    let mut view = loaded_view();
    let Some(PullsRequest::Detail(number)) = view.request.take() else {
        panic!("loading asks for the first pull request's detail");
    };
    assert_eq!(number, 14640);
    assert!(render(140, 40, &view, false).contains("Reading details…"));

    view.apply(PullsEvent::Detail(number, Ok(detail(number))));
    let screen = render(140, 40, &view, false);
    assert!(screen.contains("PULL REQUEST #14640"));
    assert!(screen.contains("open · BagToad wants feat/pr-14640 → main"));
    assert!(screen.contains("[external]"));
    assert!(screen.contains("2 passed · 1 failed · 1 running"));
    assert!(screen.contains("review required"));
    assert!(screen.contains("has conflicts with the base branch"));
    assert!(screen.contains("+40 −8 in 3 files · 3 commits · 1 comment"));
    assert!(screen.contains("Fixes the cache."));
    assert!(screen.contains("CHECKS NEEDING ATTENTION"));
    assert!(screen.contains("✕ CI / test"));
    assert!(screen.contains("◐ CI / deploy"));
    assert!(
        !screen.contains("CI / build"),
        "passing checks are not listed"
    );
    assert!(screen.contains("REVIEWS"));
    assert!(screen.contains("✓ williammartin approved · 1d ago"));
    assert!(screen.contains("✕ hubot requested changes · 2h ago"));
    assert!(screen.contains("tidy-dev was asked to review"));
    assert!(screen.contains("FILES · 2"));
    assert!(screen.contains("+30 −2  src/cache.rs"));
}

#[test]
fn long_descriptions_and_check_lists_are_cut_with_a_count() {
    let mut view = loaded_view();
    view.request = None;
    let mut long = detail(14640);
    long.body = (1..=10)
        .map(|n| format!("line {n}"))
        .collect::<Vec<_>>()
        .join("\n");
    view.apply(PullsEvent::Detail(14640, Ok(long)));
    let screen = render(140, 60, &view, false);
    assert!(screen.contains("line 6"));
    assert!(!screen.contains("line 7"));
    assert!(screen.contains("… 4 more lines on GitHub"));

    let mut many = pulls();
    many[0].checks = (0..9)
        .map(|n| check(&format!("job{n}"), CheckState::Failed))
        .collect();
    let mut view = PullsView::open(PathBuf::from("C:\\p"), None);
    view.request = None;
    view.apply(PullsEvent::Listed(PullFilter::Open, Ok(many)));
    view.request = None;
    view.apply(PullsEvent::Detail(14640, Ok(detail(14640))));
    assert!(render(140, 60, &view, false).contains("… and 3 more"));
}

#[test]
fn a_detail_that_failed_says_why() {
    let mut view = loaded_view();
    view.request = None;
    view.apply(PullsEvent::Detail(
        14640,
        Err("Could not reach GitHub.".to_owned()),
    ));
    let screen = render(140, 40, &view, true);
    assert!(screen.contains("Could not read the details."));
    assert!(screen.contains("Could not reach GitHub."));
}

#[test]
fn details_are_fetched_once_per_pull_request_and_never_replace_a_pending_action() {
    let mut view = loaded_view();
    assert_eq!(view.request.take(), Some(PullsRequest::Detail(14640)));
    view.apply(PullsEvent::Detail(14640, Ok(detail(14640))));

    key(&mut view, KeyCode::Down);
    assert_eq!(view.request.take(), Some(PullsRequest::Detail(14641)));
    view.apply(PullsEvent::Detail(14641, Ok(detail(14641))));
    key(&mut view, KeyCode::Up);
    assert_eq!(view.request, None, "14640 is already cached");

    // A refresh waiting to start must not be overwritten by moving the cursor.
    key(&mut view, KeyCode::Char('r'));
    assert_eq!(view.request, Some(PullsRequest::List(PullFilter::Open)));
    key(&mut view, KeyCode::Down);
    assert_eq!(view.request, Some(PullsRequest::List(PullFilter::Open)));
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
    assert_eq!(view.selected, 3);
    key(&mut view, KeyCode::Down);
    assert_eq!(view.selected, 3);
    key(&mut view, KeyCode::Char('g'));
    assert_eq!(view.selected, 0);
    key(&mut view, KeyCode::PageDown);
    assert_eq!(view.selected, 3);
    key(&mut view, KeyCode::PageUp);
    assert_eq!(view.selected, 0);
}

#[test]
fn enter_opens_on_github_and_c_checks_out() {
    let mut view = loaded_view();
    view.request = None;
    key(&mut view, KeyCode::Down);
    view.request = None;
    key(&mut view, KeyCode::Enter);
    assert_eq!(view.request.take(), Some(PullsRequest::Open(14641)));
    assert!(render(80, 24, &view, true).contains("opening…"));
    // One request at a time.
    key(&mut view, KeyCode::Char('c'));
    assert_eq!(view.request, None);
    assert_eq!(notice(&view), "Still working on the last request…");

    view.apply(PullsEvent::Done(Ok(
        "Opened #14641 in your browser.".to_owned()
    )));
    assert!(render(80, 24, &view, true).contains("✓ Opened #14641 in your browser."));
    key(&mut view, KeyCode::Char('c'));
    assert_eq!(view.request.take(), Some(PullsRequest::Checkout(14641)));
    assert!(render(80, 24, &view, true).contains("checking out…"));
}

#[test]
fn a_failed_checkout_reports_gits_reason() {
    let mut view = loaded_view();
    key(&mut view, KeyCode::Char('c'));
    view.request = None;
    view.apply(PullsEvent::Done(Err(
        "error: Your local changes to the following files would be overwritten by checkout"
            .to_owned(),
    )));
    assert!(view.notice.as_ref().unwrap().error);
    assert!(render(120, 24, &view, true).contains("✕ error: Your local changes"));
    key(&mut view, KeyCode::Char('c'));
    assert!(view.request.is_some(), "the screen is usable again");
}

#[test]
fn f_cycles_the_state_filter_and_drops_answers_for_a_filter_it_left() {
    let mut view = loaded_view();
    view.request = None;
    key(&mut view, KeyCode::Char('f'));
    assert_eq!(
        view.request.take(),
        Some(PullsRequest::List(PullFilter::Merged))
    );
    let screen = render(80, 24, &view, true);
    assert!(
        screen.contains("Asking GitHub…"),
        "the old list is not shown under the new filter"
    );
    assert!(screen.contains("f merged"));

    view.apply(PullsEvent::Listed(PullFilter::Open, Ok(pulls())));
    assert!(view.pulls.is_none(), "the late Open answer is ignored");
    view.apply(PullsEvent::Listed(
        PullFilter::Merged,
        Ok(vec![pull(1, "Old work")]),
    ));
    let screen = render(80, 24, &view, true);
    assert!(screen.contains("PULL REQUESTS · MERGED · 1"));
    assert!(screen.contains("#1 ● Old work"));

    for expected in [PullFilter::Closed, PullFilter::All, PullFilter::Open] {
        key(&mut view, KeyCode::Char('f'));
        assert_eq!(view.request.take(), Some(PullsRequest::List(expected)));
    }
}

#[test]
fn a_refresh_keeps_the_cursor_on_the_same_pull_request_by_number() {
    let mut view = loaded_view();
    view.request = None;
    key(&mut view, KeyCode::Char('G'));
    assert_eq!(view.selected_pull().unwrap().number, 14600);
    view.request = None;
    key(&mut view, KeyCode::Char('r'));
    assert!(
        view.details.is_empty(),
        "details may be stale after a refresh"
    );
    // The run loop starts the reload.
    assert_eq!(
        view.request.take(),
        Some(PullsRequest::List(PullFilter::Open))
    );
    let mut fresh = pulls();
    fresh.remove(0);
    view.apply(PullsEvent::Listed(PullFilter::Open, Ok(fresh)));
    assert_eq!(view.selected_pull().unwrap().number, 14600);
    assert_eq!(view.request, Some(PullsRequest::Detail(14600)));
}

#[test]
fn shows_loading_errors_and_the_empty_state_in_plain_words() {
    let mut view = PullsView::open(PathBuf::from("C:\\projects\\lumina"), None);
    view.request = None;
    assert!(render(80, 24, &view, true).contains("Asking GitHub…"));
    view.apply(PullsEvent::Listed(
        PullFilter::Open,
        Err(
            "This repository has no GitHub remote. Add one with: git remote add origin <url>."
                .to_owned(),
        ),
    ));
    let screen = render(120, 24, &view, true);
    assert!(screen.contains("Could not load pull requests."));
    assert!(screen.contains("no GitHub remote"));
    assert!(screen.contains("Press r to retry."));

    view.apply(PullsEvent::Listed(PullFilter::Open, Ok(Vec::new())));
    let screen = render(100, 24, &view, true);
    assert!(screen.contains("No open pull requests. Press f to look at another state."));
    key(&mut view, KeyCode::Enter);
    key(&mut view, KeyCode::Char('c'));
    key(&mut view, KeyCode::Down);
    assert!(view.request.is_none(), "nothing to act on");
    assert!(view.busy.is_none());
}

#[test]
fn number_keys_switch_screens_and_tab_swaps_list_and_detail() {
    let mut view = loaded_view();
    key(&mut view, KeyCode::Char('4'));
    assert!(!view.closed && view.goto.is_none(), "already here");
    key(&mut view, KeyCode::Char('2'));
    assert_eq!(view.goto.take(), Some(Screen::Commits));
    key(&mut view, KeyCode::Char('3'));
    assert_eq!(view.goto.take(), Some(Screen::Branches));
    key(&mut view, KeyCode::Char('1'));
    assert!(view.closed);

    let mut view = loaded_view();
    key(&mut view, KeyCode::Tab);
    let screen = render(80, 24, &view, true);
    assert!(screen.contains("PULL REQUEST #14640"));
    assert!(!screen.contains("PULL REQUESTS ·"));
    assert!(screen.contains("esc List"));
    key(&mut view, KeyCode::Esc);
    assert!(!view.closed);
    key(&mut view, KeyCode::Esc);
    assert!(view.closed);
}

#[test]
fn quit_keys_and_works_without_color() {
    let mut view = loaded_view();
    key(&mut view, KeyCode::Char('q'));
    assert!(view.should_quit);
    let mut view = loaded_view();
    view.handle_key(KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert!(view.should_quit);
    assert_eq!(
        view.request,
        Some(PullsRequest::Detail(14640)),
        "ctrl+c is not checkout"
    );
    let screen = render(80, 24, &loaded_view(), false);
    assert!(screen.contains("> #14640 ●"));
    assert!(screen.contains("✕ 1/4"));
    assert!(screen.contains("○ WIP"));
}
