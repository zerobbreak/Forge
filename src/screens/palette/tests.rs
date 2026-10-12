use super::{Action, Outcome, PaletteView, commands, draw_palette};
use crate::{screens::Screen, theme::Theme};
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::{Terminal, backend::TestBackend};

fn render(width: u16, height: u16, view: &PaletteView, colors: bool) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
    terminal
        .draw(|frame| draw_palette(frame, view, &Theme::new(colors)))
        .expect("palette renders");
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

fn press(view: &mut PaletteView, code: KeyCode) {
    view.handle_key(code, KeyModifiers::NONE);
}

fn type_text(view: &mut PaletteView, text: &str) {
    for character in text.chars() {
        press(view, KeyCode::Char(character));
    }
}

fn titles(view: &PaletteView) -> Vec<&'static str> {
    view.matching()
        .iter()
        .map(|command| command.title)
        .collect()
}

#[test]
fn every_screen_lists_its_own_commands_before_the_shared_ones() {
    for (screen, first) in [
        (Screen::Changes, "Stage hunk"),
        (Screen::Commits, "Toggle all branches / current branch"),
        (Screen::Branches, "Switch to the selected branch"),
        (Screen::PullRequests, "Open on GitHub"),
    ] {
        let list = commands(screen);
        assert_eq!(list[0].title, first, "{screen:?}");
        assert_eq!(list.last().unwrap().action, Action::Quit);
        assert!(list.iter().any(|c| c.action == Action::Projects));
        for (target, _) in Screen::TABS {
            assert!(
                list.iter().any(|c| c.action == Action::Screen(target)),
                "{screen:?} can go to {target:?}"
            );
        }
        // Every command shows the key that does the same thing.
        assert!(list.iter().all(|c| !c.keys.is_empty()));
    }
}

#[test]
fn typing_narrows_the_list_and_matches_keywords_in_any_order() {
    let mut view = PaletteView::open(Screen::Branches);
    let all = titles(&view).len();
    type_text(&mut view, "branch");
    let narrowed = titles(&view);
    assert!(narrowed.len() < all);
    assert!(narrowed.contains(&"New branch from the selected one…"));
    assert!(narrowed.contains(&"Delete branch…"));
    assert!(narrowed.contains(&"Go to Branches"));
    assert!(!narrowed.contains(&"Quit Forge"));

    let mut view = PaletteView::open(Screen::Changes);
    type_text(&mut view, "index add");
    assert_eq!(titles(&view), ["Stage hunk", "Stage file"]);
    let mut view = PaletteView::open(Screen::PullRequests);
    type_text(&mut view, "checkout");
    assert_eq!(titles(&view)[0], "Check out this pull request");
    let mut view = PaletteView::open(Screen::Changes);
    type_text(&mut view, "COMMIT");
    assert_eq!(titles(&view)[0], "Commit…", "case does not matter");
}

#[test]
fn keywords_find_commands_whose_titles_do_not_say_the_word() {
    let mut view = PaletteView::open(Screen::Changes);
    type_text(&mut view, "log");
    assert_eq!(titles(&view), ["Go to Commits"]);
    let mut view = PaletteView::open(Screen::Changes);
    type_text(&mut view, "history");
    assert_eq!(titles(&view), ["Go to Commits"]);
    let mut view = PaletteView::open(Screen::Commits);
    type_text(&mut view, "refresh");
    assert_eq!(titles(&view)[0], "Refresh");
}

#[test]
fn backspace_widens_the_list_again_and_the_choice_returns_to_the_top() {
    let mut view = PaletteView::open(Screen::Changes);
    let all = titles(&view).len();
    type_text(&mut view, "zzz");
    assert!(titles(&view).is_empty());
    for _ in 0..3 {
        press(&mut view, KeyCode::Backspace);
    }
    assert_eq!(titles(&view).len(), all);
    assert_eq!(view.selected, 0);
}

#[test]
fn arrows_move_the_choice_within_the_matches_and_enter_runs_it() {
    let mut view = PaletteView::open(Screen::Changes);
    press(&mut view, KeyCode::Up);
    assert_eq!(view.selected, 0);
    press(&mut view, KeyCode::Down);
    press(&mut view, KeyCode::Tab);
    assert_eq!(view.selected, 2);
    press(&mut view, KeyCode::BackTab);
    assert_eq!(view.selected, 1);
    press(&mut view, KeyCode::Enter);
    assert_eq!(
        view.outcome.take(),
        Some(Outcome::Run(Action::Key(KeyCode::Char('a'))))
    );

    let mut view = PaletteView::open(Screen::Changes);
    press(&mut view, KeyCode::PageDown);
    assert_eq!(view.selected, 5);
    for _ in 0..40 {
        press(&mut view, KeyCode::Down);
    }
    assert_eq!(
        view.selected,
        view.matches.len() - 1,
        "stops at the last match"
    );
    press(&mut view, KeyCode::PageUp);
    assert_eq!(view.selected, view.matches.len() - 6);
}

#[test]
fn enter_with_no_match_does_nothing_and_escape_closes() {
    let mut view = PaletteView::open(Screen::Changes);
    type_text(&mut view, "nothing like this");
    press(&mut view, KeyCode::Enter);
    assert_eq!(view.outcome, None);
    press(&mut view, KeyCode::Esc);
    assert_eq!(view.outcome.take(), Some(Outcome::Close));

    let mut view = PaletteView::open(Screen::Changes);
    view.handle_key(KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert_eq!(view.outcome.take(), Some(Outcome::Close));
}

#[test]
fn slash_and_other_punctuation_are_just_text() {
    let mut view = PaletteView::open(Screen::Changes);
    type_text(&mut view, "/ x");
    assert_eq!(view.query(), "/ x");
}

#[test]
fn renders_the_commands_with_their_keys_at_80x24() {
    let view = PaletteView::open(Screen::Branches);
    let screen = render(80, 24, &view, true);
    assert!(screen.contains("COMMANDS · BRANCHES"));
    assert!(screen.contains("/ "));
    assert!(screen.contains("> Switch to the selected branch"));
    assert!(screen.contains("enter"));
    assert!(screen.contains("Delete branch…"));
    assert!(screen.contains("Go to Pull Requests"));
    assert!(screen.contains("Run"));
    assert!(screen.contains("esc Close"));
}

#[test]
fn renders_the_query_and_a_helpful_message_when_nothing_matches() {
    let mut view = PaletteView::open(Screen::PullRequests);
    type_text(&mut view, "zzz");
    let screen = render(80, 24, &view, false);
    assert!(screen.contains("/ zzz"));
    assert!(screen.contains("No command matches “zzz”. Clear the text to see them all."));
    assert!(
        render(40, 12, &view, false).contains("COMMANDS"),
        "fits a small terminal"
    );
}

#[test]
fn works_without_color_and_marks_the_choice() {
    let mut view = PaletteView::open(Screen::Commits);
    press(&mut view, KeyCode::Down);
    let screen = render(100, 30, &view, false);
    assert!(screen.contains("> Newest commit"));
    assert!(!screen.contains("> Toggle all branches"));
}
