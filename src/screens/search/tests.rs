use super::{Focus, SearchEvent, SearchRequest, SearchView, draw_search};
use crate::{
    git::{
        repository::BranchTracking,
        search::{
            Context, Match, ReplaceReport, SearchResult, read_context, replace_matches, search,
        },
    },
    screens::Screen,
    test_support::{TestDirectory, git_text, run_git},
    theme::Theme,
};
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::{Terminal, backend::TestBackend};
use std::{fs, path::PathBuf};

fn found(path: &str, line: usize, text: &str) -> Match {
    Match {
        path: path.to_owned(),
        line,
        text: text.to_owned(),
    }
}

fn sample() -> Vec<Match> {
    vec![
        found("docs/readme.md", 3, "The old_name function"),
        found("src/app.rs", 12, "    let value = old_name();"),
        found("src/app.rs", 40, "fn old_name() {}"),
        found("src/ui.rs", 7, "use old_name::thing;"),
    ]
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

fn new_view() -> SearchView {
    SearchView::open(PathBuf::from("C:\\projects\\lumina"), Some(tracking()))
}

fn key(view: &mut SearchView, code: KeyCode) {
    view.handle_key(code, KeyModifiers::NONE);
}

fn alt(view: &mut SearchView, letter: char) {
    view.handle_key(KeyCode::Char(letter), KeyModifiers::ALT);
}

fn type_text(view: &mut SearchView, text: &str) {
    for character in text.chars() {
        key(view, KeyCode::Char(character));
    }
}

/// Takes the pending search request and answers it with `matches`.
fn answer(view: &mut SearchView, matches: Vec<Match>) {
    let Some(SearchRequest::Search { id, .. }) = view.request.take() else {
        panic!("a search was requested");
    };
    view.apply(SearchEvent::Searched {
        id,
        result: Ok(SearchResult {
            matches,
            truncated: false,
        }),
    });
}

fn context_for(first_line: usize, lines: &[&str]) -> Context {
    Context {
        first_line,
        lines: lines.iter().map(|line| (*line).to_owned()).collect(),
    }
}

/// A view that searched "old_name" and shows the sample results, cursor on the first,
/// focus on the results.
fn results_view() -> SearchView {
    let mut view = new_view();
    type_text(&mut view, "old_name");
    answer(&mut view, sample());
    key(&mut view, KeyCode::Enter);
    assert_eq!(view.focus, Focus::Results);
    view.request = None;
    view
}

fn render(width: u16, height: u16, view: &SearchView, colors: bool) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
    terminal
        .draw(|frame| draw_search(frame, view, &Theme::new(colors), Some("unathi")))
        .expect("search screen renders");
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

fn notice(view: &SearchView) -> String {
    view.notice
        .as_ref()
        .map(|n| n.text.clone())
        .unwrap_or_default()
}

#[test]
fn searching_starts_at_two_characters_and_each_keystroke_supersedes_the_last() {
    let mut view = new_view();
    type_text(&mut view, "o");
    assert_eq!(view.request, None, "one letter matches nearly every line");
    type_text(&mut view, "l");
    let Some(SearchRequest::Search {
        id: first, query, ..
    }) = view.request.clone()
    else {
        panic!("two characters start a search");
    };
    assert_eq!(query, "ol");
    type_text(&mut view, "d");
    let Some(SearchRequest::Search {
        id: second, query, ..
    }) = view.request.clone()
    else {
        panic!("typing again replaces the pending search");
    };
    assert_eq!(query, "old");
    assert!(second > first);

    // The answer to the first search arrives late and is ignored.
    view.apply(SearchEvent::Searched {
        id: first,
        result: Ok(SearchResult {
            matches: sample(),
            truncated: false,
        }),
    });
    assert!(view.matches.is_empty());
    assert!(view.searching);
    view.apply(SearchEvent::Searched {
        id: second,
        result: Ok(SearchResult {
            matches: sample(),
            truncated: false,
        }),
    });
    assert_eq!(view.matches.len(), 4);
    assert!(!view.searching);

    // Deleting back below two characters clears the results and cancels any pending search.
    key(&mut view, KeyCode::Backspace);
    key(&mut view, KeyCode::Backspace);
    assert!(view.matches.is_empty());
    assert_eq!(view.request, None);
}

#[test]
fn a_late_answer_after_the_query_was_cleared_is_dropped() {
    let mut view = new_view();
    type_text(&mut view, "old");
    let Some(SearchRequest::Search { id, .. }) = view.request.take() else {
        panic!("searching");
    };
    for _ in 0..3 {
        key(&mut view, KeyCode::Backspace);
    }
    view.apply(SearchEvent::Searched {
        id,
        result: Ok(SearchResult {
            matches: sample(),
            truncated: false,
        }),
    });
    assert!(view.matches.is_empty());
}

#[test]
fn renders_boxes_options_and_grouped_results_at_80x24() {
    let view = results_view();
    let screen = render(80, 24, &view, true);
    assert!(screen.contains("[5 Search]"));
    assert!(screen.contains("  Find    old_name"));
    assert!(screen.contains("  Replace"));
    assert!(screen.contains(" Aa "), "options are off by default");
    assert!(screen.contains("4 matches in 3 files"));
    assert!(
        screen.contains("▸ RESULTS · 4 IN 3 FILES"),
        "results have focus"
    );
    assert!(screen.contains("docs/readme.md  1"));
    assert!(screen.contains("src/app.rs  2"));
    assert!(screen.contains("> "), "the cursor row is marked");
    assert!(screen.contains("    3  The old_name function"));
    assert!(
        screen.contains("   12  let value = old_name();"),
        "indentation is trimmed"
    );
    assert!(screen.contains("Skip match"));
    assert!(screen.contains("Replace with…"), "no replacement typed yet");
    assert!(
        !screen.contains("[x]"),
        "checkboxes only matter once there is a replacement"
    );
}

#[test]
fn the_focused_box_is_marked_and_the_cursor_follows_it() {
    let mut view = new_view();
    assert!(render(80, 24, &view, false).contains("▸ Find"));
    key(&mut view, KeyCode::Tab);
    assert_eq!(view.focus, Focus::Replace);
    let screen = render(80, 24, &view, false);
    assert!(screen.contains("▸ Replace"));
    assert!(screen.contains("Next box"));
}

#[test]
fn tab_cycles_the_boxes_and_enter_or_down_moves_to_the_results() {
    let mut view = new_view();
    key(&mut view, KeyCode::Tab);
    assert_eq!(view.focus, Focus::Replace);
    key(&mut view, KeyCode::Tab);
    assert_eq!(view.focus, Focus::Results);
    key(&mut view, KeyCode::Tab);
    assert_eq!(view.focus, Focus::Find);
    key(&mut view, KeyCode::BackTab);
    assert_eq!(view.focus, Focus::Results);
    key(&mut view, KeyCode::BackTab);
    assert_eq!(view.focus, Focus::Replace);
    key(&mut view, KeyCode::BackTab);
    assert_eq!(view.focus, Focus::Find);
    key(&mut view, KeyCode::Down);
    assert_eq!(view.focus, Focus::Results);
    key(&mut view, KeyCode::Char('i'));
    assert_eq!(view.focus, Focus::Find);
    key(&mut view, KeyCode::Enter);
    assert_eq!(view.focus, Focus::Results);
}

#[test]
fn keys_are_text_in_the_boxes_and_commands_in_the_results() {
    let mut view = new_view();
    type_text(&mut view, "q1/ x");
    assert_eq!(
        view.query(),
        "q1/ x",
        "digits, q and / are text while typing"
    );
    assert!(!view.should_quit && view.goto.is_none() && !view.closed && !view.open_palette);
    key(&mut view, KeyCode::Tab);
    type_text(&mut view, "5");
    assert_eq!(view.replacement(), "5");

    let mut view = results_view();
    key(&mut view, KeyCode::Char('/'));
    assert!(view.open_palette);
    key(&mut view, KeyCode::Char('3'));
    assert_eq!(view.goto, Some(Screen::Branches));
    view.goto = None;
    key(&mut view, KeyCode::Char('5'));
    assert_eq!(view.goto, None, "already on Search");
    key(&mut view, KeyCode::Char('1'));
    assert!(view.closed);
    let mut view = results_view();
    key(&mut view, KeyCode::Char('q'));
    assert!(view.should_quit);
}

#[test]
fn escape_leaves_from_the_boxes_and_the_results() {
    let mut view = new_view();
    key(&mut view, KeyCode::Esc);
    assert!(view.closed);
    let mut view = results_view();
    key(&mut view, KeyCode::Esc);
    assert!(view.closed);
    let mut view = new_view();
    view.handle_key(KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert!(view.should_quit);
}

#[test]
fn option_toggles_work_with_alt_in_the_boxes_and_with_a_letter_in_the_results() {
    let mut view = new_view();
    type_text(&mut view, "old_name");
    view.request = None;
    alt(&mut view, 'c');
    assert!(view.options.case_sensitive);
    assert_eq!(view.query(), "old_name", "alt+c is not text");
    let Some(SearchRequest::Search { options, .. }) = view.request.take() else {
        panic!("changing an option searches again");
    };
    assert!(options.case_sensitive);
    assert!(render(80, 24, &view, false).contains("[Aa]"));

    alt(&mut view, 'e');
    alt(&mut view, 'w');
    assert!(view.options.regex && view.options.whole_word);
    let screen = render(80, 24, &view, false);
    assert!(screen.contains("[.*]") && screen.contains("[W]"));

    let mut view = results_view();
    key(&mut view, KeyCode::Char('c'));
    key(&mut view, KeyCode::Char('e'));
    key(&mut view, KeyCode::Char('w'));
    assert!(view.options.case_sensitive && view.options.regex && view.options.whole_word);
    assert!(matches!(view.request, Some(SearchRequest::Search { .. })));
    key(&mut view, KeyCode::Char('c'));
    assert!(!view.options.case_sensitive);
}

#[test]
fn an_invalid_pattern_is_explained_and_nothing_is_searched() {
    let mut view = new_view();
    alt(&mut view, 'e');
    type_text(&mut view, "(oops");
    assert_eq!(
        view.request, None,
        "git is never asked to run a bad pattern"
    );
    let screen = render(100, 24, &view, true);
    assert!(screen.contains("That is not a valid pattern"), "{screen}");
    // Fixing it searches again.
    key(&mut view, KeyCode::Char(')'));
    assert!(matches!(view.request, Some(SearchRequest::Search { .. })));
}

#[test]
fn shows_hints_searching_empty_and_error_states_in_plain_words() {
    let mut view = new_view();
    assert!(render(100, 24, &view, true).contains("Type at least two characters"));
    type_text(&mut view, "zz");
    assert!(render(100, 24, &view, true).contains("Searching…"));
    answer(&mut view, Vec::new());
    let screen = render(100, 24, &view, true);
    assert!(screen.contains("No matches for “zz”."));
    assert!(screen.contains("Ignored and binary files are not searched."));

    type_text(&mut view, "z");
    let Some(SearchRequest::Search { id, .. }) = view.request.take() else {
        panic!("searching");
    };
    view.apply(SearchEvent::Searched {
        id,
        result: Err("Git could not search for that (boom).".to_owned()),
    });
    assert!(render(100, 24, &view, true).contains("Git could not search for that (boom)."));
}

#[test]
fn the_cursor_moves_through_matches_and_asks_for_context() {
    let mut view = results_view();
    key(&mut view, KeyCode::Char('j'));
    assert_eq!(view.selected, 1);
    assert_eq!(
        view.request.take(),
        Some(SearchRequest::Context {
            path: "src/app.rs".to_owned(),
            line: 12
        })
    );
    key(&mut view, KeyCode::Down);
    key(&mut view, KeyCode::Down);
    key(&mut view, KeyCode::Down);
    assert_eq!(view.selected, 3, "stops at the last match");
    key(&mut view, KeyCode::Char('g'));
    assert_eq!(view.selected, 0);
    key(&mut view, KeyCode::End);
    assert_eq!(view.selected, 3);
    key(&mut view, KeyCode::PageUp);
    assert_eq!(view.selected, 0);
    key(&mut view, KeyCode::PageDown);
    assert_eq!(view.selected, 3);
    key(&mut view, KeyCode::Up);
    assert_eq!(view.selected, 2);
}

#[test]
fn the_preview_shows_context_and_a_before_and_after_when_replacing() {
    let mut view = results_view();
    key(&mut view, KeyCode::Down);
    let Some(SearchRequest::Context { path, line }) = view.request.take() else {
        panic!("context requested");
    };
    // A context for a match the cursor already left is dropped.
    view.apply(SearchEvent::Context {
        path: "docs/readme.md".to_owned(),
        line: 3,
        result: Ok(context_for(1, &["stale"])),
    });
    assert!(render(140, 40, &view, false).contains("Reading file…"));

    view.apply(SearchEvent::Context {
        path,
        line,
        result: Ok(context_for(
            10,
            &[
                "fn setup() {",
                "    init();",
                "    let value = old_name();",
                "    run(value);",
                "}",
            ],
        )),
    });
    let screen = render(140, 40, &view, false);
    assert!(screen.contains("src/app.rs:12"));
    assert!(screen.contains("    10  fn setup() {") || screen.contains("   10  fn setup() {"));
    assert!(screen.contains("▌  12      let value = old_name();") || screen.contains("▌  12"));
    assert!(screen.contains("run(value);"));
    assert!(!screen.contains("stale"));

    // With a replacement the matched line is shown as removed and added.
    key(&mut view, KeyCode::Char('i'));
    key(&mut view, KeyCode::Tab);
    type_text(&mut view, "new_name");
    let screen = render(140, 40, &view, false);
    assert!(screen.contains("-  12      let value = old_name();") || screen.contains("-  12"));
    assert!(screen.contains("let value = new_name();"));
    assert!(screen.contains("+  12") || screen.contains("+  12 "));
}

#[test]
fn a_preview_that_failed_says_why() {
    let mut view = results_view();
    view.apply(SearchEvent::Context {
        path: "docs/readme.md".to_owned(),
        line: 3,
        result: Err("docs/readme.md is too large to preview.".to_owned()),
    });
    let screen = render(140, 40, &view, false);
    assert!(screen.contains("Could not show this file."));
    assert!(screen.contains("large to preview"));
}

#[test]
fn long_lines_scroll_so_the_match_stays_visible() {
    let mut view = new_view();
    type_text(&mut view, "needle");
    let long = format!("{} needle {}", "x".repeat(150), "y".repeat(50));
    answer(&mut view, vec![found("a.txt", 1, &long)]);
    let screen = render(80, 24, &view, false);
    assert!(screen.contains("…"), "the start is cut");
    assert!(screen.contains("needle"), "{screen}");
}

#[test]
fn space_and_x_leave_matches_out_of_the_replacement() {
    let mut view = view_ready_to_replace();
    assert_eq!(view.focus, Focus::Results);
    let screen = render(100, 24, &view, false);
    assert!(screen.contains("[x]"));
    assert!(screen.contains("4 matches in 3 files · 4 to replace"));

    key(&mut view, KeyCode::Char(' '));
    let screen = render(100, 24, &view, false);
    assert!(screen.contains("[ ]"));
    assert!(screen.contains("· 3 to replace"));
    key(&mut view, KeyCode::Char(' '));
    assert!(render(100, 24, &view, false).contains("· 4 to replace"));

    // x leaves the whole file out, and brings it back when pressed again.
    key(&mut view, KeyCode::Char('j'));
    key(&mut view, KeyCode::Char('x'));
    assert!(render(100, 24, &view, false).contains("· 2 to replace"));
    key(&mut view, KeyCode::Char('x'));
    assert!(render(100, 24, &view, false).contains("· 4 to replace"));
}

fn view_ready_to_replace() -> SearchView {
    let mut view = results_view();
    key(&mut view, KeyCode::Char('i'));
    key(&mut view, KeyCode::Tab);
    type_text(&mut view, "new_name");
    key(&mut view, KeyCode::Tab);
    view.request = None;
    view
}

#[test]
fn replacing_needs_a_replacement_first() {
    let mut view = results_view();
    key(&mut view, KeyCode::Char('R'));
    assert!(notice(&view).contains("Type what to replace it with first"));
    key(&mut view, KeyCode::Enter);
    assert!(view.confirm.is_none());
    assert!(view.request.is_none());
}

#[test]
fn replace_all_asks_first_and_only_y_confirms() {
    let mut view = view_ready_to_replace();
    key(&mut view, KeyCode::Char(' '));
    key(&mut view, KeyCode::Char('R'));
    assert!(
        view.request.is_none(),
        "nothing runs before the user confirms"
    );
    let screen = render(100, 30, &view, true);
    assert!(screen.contains("REPLACE"));
    assert!(screen.contains("Replace in 3 lines across 2 files?"));
    assert!(screen.contains("“old_name” → “new_name”"));
    assert!(screen.contains("src/app.rs:12"));
    assert!(screen.contains("- let value = old_name();"));
    assert!(screen.contains("+ let value = new_name();"));
    assert!(screen.contains("refs/forge/snapshots/"));
    assert!(screen.contains("y Replace") || screen.contains("Replace"));

    for code in [KeyCode::Enter, KeyCode::Char('R'), KeyCode::Char('x')] {
        key(&mut view, code);
        assert!(view.request.is_none(), "{code:?} must not confirm");
        assert!(view.confirm.is_some());
    }
    key(&mut view, KeyCode::Esc);
    assert!(view.confirm.is_none() && view.request.is_none());

    key(&mut view, KeyCode::Char('R'));
    key(&mut view, KeyCode::Char('n'));
    assert!(view.confirm.is_none() && view.request.is_none());

    key(&mut view, KeyCode::Char('R'));
    key(&mut view, KeyCode::Char('y'));
    let Some(SearchRequest::Replace {
        query,
        replacement,
        targets,
        ..
    }) = view.request.take()
    else {
        panic!("y starts the replacement");
    };
    assert_eq!(
        (query.as_str(), replacement.as_str()),
        ("old_name", "new_name")
    );
    assert_eq!(
        targets,
        vec![
            found("src/app.rs", 12, "    let value = old_name();"),
            found("src/app.rs", 40, "fn old_name() {}"),
            found("src/ui.rs", 7, "use old_name::thing;"),
        ],
        "the match left out stays untouched"
    );
    assert!(render(100, 24, &view, true).contains("replacing…"));
    // Not twice at once.
    key(&mut view, KeyCode::Char('R'));
    assert_eq!(notice(&view), "Still replacing…");
}

#[test]
fn enter_replaces_only_the_selected_match() {
    let mut view = view_ready_to_replace();
    key(&mut view, KeyCode::Char('j'));
    view.request = None;
    key(&mut view, KeyCode::Enter);
    let screen = render(100, 30, &view, true);
    assert!(screen.contains("Replace in 1 line across 1 file?"));
    key(&mut view, KeyCode::Char('y'));
    let Some(SearchRequest::Replace { targets, .. }) = view.request.take() else {
        panic!("replace requested");
    };
    assert_eq!(
        targets,
        vec![found("src/app.rs", 12, "    let value = old_name();")]
    );
}

#[test]
fn replacing_is_refused_with_nothing_included_or_while_results_are_out_of_date() {
    let mut view = view_ready_to_replace();
    for _ in 0..4 {
        key(&mut view, KeyCode::Char(' '));
        key(&mut view, KeyCode::Char('j'));
    }
    key(&mut view, KeyCode::Char('R'));
    assert!(notice(&view).contains("Every match is left out"));
    assert!(view.confirm.is_none());

    // Editing the search makes the shown matches stale until the new answer arrives.
    let mut view = view_ready_to_replace();
    key(&mut view, KeyCode::Char('i'));
    type_text(&mut view, "x");
    key(&mut view, KeyCode::Tab);
    key(&mut view, KeyCode::Tab);
    key(&mut view, KeyCode::Char('R'));
    assert!(notice(&view).contains("still catching up"));
    assert!(view.confirm.is_none());
}

#[test]
fn a_finished_replace_reports_the_undo_and_searches_again() {
    let mut view = view_ready_to_replace();
    view.excluded.insert(("src/ui.rs".to_owned(), 7));
    view.apply(SearchEvent::Replaced(Ok(ReplaceReport {
        lines: 3,
        files: 2,
        changed_since_search: 1,
        skipped_files: Vec::new(),
        snapshots: vec![
            (
                "src/app.rs".to_owned(),
                "refs/forge/snapshots/111".to_owned(),
            ),
            (
                "docs/readme.md".to_owned(),
                "refs/forge/snapshots/112".to_owned(),
            ),
        ],
    })));
    let text = notice(&view);
    assert!(text.starts_with("Replaced 3 lines in 2 files."), "{text}");
    assert!(text.contains("1 left alone because the file changed"));
    assert!(text.contains("Undo: git checkout refs/forge/snapshots/111 -- src/app.rs"));
    assert!(text.contains("(+1 more under refs/forge/snapshots)"));
    assert!(view.excluded.is_empty());
    assert!(
        matches!(view.request, Some(SearchRequest::Search { .. })),
        "the results are refreshed"
    );

    let mut view = view_ready_to_replace();
    view.apply(SearchEvent::Replaced(Err(
        "Could not write src/app.rs: access denied.".to_owned(),
    )));
    assert!(view.notice.as_ref().unwrap().error);
    assert!(notice(&view).starts_with("Nothing more was replaced. Could not write"));

    let mut view = view_ready_to_replace();
    view.apply(SearchEvent::Replaced(Ok(ReplaceReport {
        changed_since_search: 2,
        ..ReplaceReport::default()
    })));
    assert!(view.notice.as_ref().unwrap().error);
    assert!(notice(&view).starts_with("Nothing was replaced."));
}

#[test]
fn a_refresh_keeps_the_cursor_and_the_left_out_matches_for_the_same_search() {
    let mut view = results_view();
    key(&mut view, KeyCode::Char('j'));
    key(&mut view, KeyCode::Char(' '));
    view.request = None;
    key(&mut view, KeyCode::Char('r'));
    let mut after = sample();
    after.remove(0);
    answer(&mut view, after);
    assert_eq!(
        view.selected_match().unwrap().line,
        12,
        "same match, new position"
    );
    assert_eq!(
        view.excluded.len(),
        1,
        "same search keeps what was left out"
    );

    // A different query starts over.
    key(&mut view, KeyCode::Char('i'));
    type_text(&mut view, "s");
    answer(&mut view, sample());
    assert!(view.excluded.is_empty());
}

#[test]
fn works_without_color_and_marks_the_chosen_row() {
    let view = results_view();
    let screen = render(80, 24, &view, false);
    assert!(screen.contains("> "));
    assert!(screen.contains("▸ RESULTS"));
}

#[test]
fn searches_replaces_and_undoes_in_a_real_repository() {
    let directory = TestDirectory::new("search-screen");
    let repo = directory.path().join("repo");
    fs::create_dir_all(repo.join("src")).unwrap();
    run_git(&repo, &["init", "-b", "main"]);
    run_git(&repo, &["config", "user.name", "Forge Test"]);
    run_git(&repo, &["config", "user.email", "forge@example.test"]);
    run_git(&repo, &["config", "commit.gpgsign", "false"]);
    run_git(&repo, &["config", "core.autocrlf", "false"]);
    fs::write(
        repo.join("src/a.rs"),
        "let old_name = 1;\nlet keep = 2;\nold_name();\n",
    )
    .unwrap();
    fs::write(repo.join("b.txt"), "the old_name here\n").unwrap();
    run_git(&repo, &["add", "."]);
    run_git(&repo, &["commit", "-m", "files"]);

    let mut view = SearchView::open(repo.clone(), None);
    // The run loop's work, done inline.
    let settle = |view: &mut SearchView| {
        while let Some(request) = view.request.take() {
            match request {
                SearchRequest::Search { id, query, options } => {
                    let result = search(&repo, &query, &options);
                    view.apply(SearchEvent::Searched { id, result });
                }
                SearchRequest::Context { path, line } => {
                    let result = read_context(&repo, &path, line, 4);
                    view.apply(SearchEvent::Context { path, line, result });
                }
                SearchRequest::Replace {
                    query,
                    replacement,
                    options,
                    targets,
                } => {
                    let result = replace_matches(&repo, &query, &replacement, &options, &targets);
                    view.apply(SearchEvent::Replaced(result));
                }
            }
        }
    };

    type_text(&mut view, "old_name");
    settle(&mut view);
    assert_eq!(view.matches.len(), 3);
    key(&mut view, KeyCode::Enter);
    // Results are in path order, so b.txt is first; move to src/a.rs to see its neighbours.
    key(&mut view, KeyCode::Down);
    settle(&mut view);
    let screen = render(140, 40, &view, true);
    assert!(screen.contains("b.txt"));
    assert!(screen.contains("src/a.rs  2"));
    assert!(
        screen.contains("let keep = 2;"),
        "the preview shows surrounding lines"
    );

    key(&mut view, KeyCode::Char('i'));
    key(&mut view, KeyCode::Tab);
    type_text(&mut view, "fresh_name");
    key(&mut view, KeyCode::Tab);
    assert_eq!(view.focus, Focus::Results);
    // Leave b.txt out, then replace the rest.
    key(&mut view, KeyCode::Up);
    key(&mut view, KeyCode::Char('x'));
    key(&mut view, KeyCode::Char('R'));
    key(&mut view, KeyCode::Char('y'));
    settle(&mut view);

    assert_eq!(
        fs::read_to_string(repo.join("src/a.rs")).unwrap(),
        "let fresh_name = 1;\nlet keep = 2;\nfresh_name();\n"
    );
    assert_eq!(
        fs::read_to_string(repo.join("b.txt")).unwrap(),
        "the old_name here\n",
        "the file that was left out is untouched"
    );
    let text = notice(&view);
    assert!(text.starts_with("Replaced 2 lines in 1 file."), "{text}");
    assert!(
        view.matches.iter().all(|m| m.path == "b.txt"),
        "the list refreshed"
    );

    // The undo command in the notice works.
    let snapshot = text
        .split("git checkout ")
        .nth(1)
        .and_then(|rest| rest.split_whitespace().next())
        .expect("the notice names a snapshot");
    run_git(&repo, &["checkout", snapshot, "--", "src/a.rs"]);
    assert_eq!(
        fs::read_to_string(repo.join("src/a.rs")).unwrap(),
        "let old_name = 1;\nlet keep = 2;\nold_name();\n"
    );
    assert_eq!(
        git_text(&repo, &["for-each-ref", "refs/forge/snapshots"])
            .lines()
            .count(),
        1
    );
}
