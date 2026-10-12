use super::{Confirm, Focus, SearchView};
use crate::{
    editor::TextArea,
    git::search::{Match, find_spans, replace_in_line},
    screens::Screen,
    theme::{Role, Theme},
    ui::{
        Hint, draw_bar, draw_dialog, draw_tabs, draw_title_bar, fit, hint, key_line, pane_block,
        primary, repo_context, ruled_heading, status_line, window,
    },
};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Position, Rect},
    style::Modifier,
    text::{Line, Span},
    widgets::{List, ListItem, ListState, Paragraph, Wrap},
};
use regex::Regex;
use std::ops::Range;

/// At this width the preview sits beside the results; below it only the results show.
const SPLIT_MIN_WIDTH: u16 = 100;
const LABEL_WIDTH: usize = 10;
/// Characters of a result line shown before the match is scrolled into view.
const LEAD: usize = 12;

pub fn draw_search(frame: &mut Frame<'_>, view: &SearchView, theme: &Theme, user: Option<&str>) {
    let outer = window(theme);
    let inner = outer.inner(frame.area());
    frame.render_widget(outer, frame.area());

    let busy = view
        .busy
        .or(view.searching.then_some("searching…"))
        .filter(|_| view.confirm.is_none());
    let notice = status_line(theme, busy, view.notice.as_ref());
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(2),
            Constraint::Min(3),
            Constraint::Length(u16::from(notice.is_some())),
            Constraint::Length(1),
        ])
        .split(inner);
    let context = repo_context(theme, &view.path, view.tracking.as_ref());
    draw_title_bar(frame, theme, rows[0], context, user);
    draw_tabs(frame, theme, rows[1], Screen::Search);
    draw_inputs(frame, view, theme, rows[2]);
    draw_body(frame, view, theme, rows[3]);
    if let Some(notice) = notice {
        frame.render_widget(Paragraph::new(notice), rows[4]);
    }
    let width = usize::from(rows[5].width);
    draw_bar(frame, theme, rows[5], key_line(theme, &hints(view), width));

    if let Some(confirm) = &view.confirm {
        draw_confirm(frame, view, theme, confirm, inner);
    }
}

fn hints(view: &SearchView) -> Vec<Hint> {
    match view.focus {
        Focus::Find | Focus::Replace => vec![
            primary("enter", "Results"),
            hint("tab", "Next box"),
            hint("alt+c", "Case"),
            hint("alt+e", "Regex"),
            hint("alt+w", "Word"),
            hint("esc", "Changes"),
        ],
        Focus::Results => {
            let mut hints = vec![hint("tab", "Edit search"), hint("space", "Skip match")];
            if view.replacement().is_empty() {
                hints.push(hint("tab tab", "Replace with…"));
            } else {
                hints.extend([hint("enter", "Replace this"), primary("R", "Replace all")]);
            }
            hints.extend([
                hint("x", "Skip file"),
                hint("c e w", "Options"),
                hint("/", "Commands"),
                hint("esc", "Changes"),
                hint("q", "Quit"),
            ]);
            hints
        }
    }
}

/// The Find and Replace boxes, with the option chips and a count beside them.
fn draw_inputs(frame: &mut Frame<'_>, view: &SearchView, theme: &Theme, area: Rect) {
    let lines = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(area);

    let chip = |on: bool, label: &str| {
        if on {
            Span::styled(format!("[{label}]"), theme.bold(Role::Cursor))
        } else {
            Span::styled(format!(" {label} "), theme.fg(Role::Muted))
        }
    };
    let chips = Line::from(vec![
        chip(view.options.case_sensitive, "Aa"),
        Span::raw(" "),
        chip(view.options.regex, ".*"),
        Span::raw(" "),
        chip(view.options.whole_word, "W"),
        Span::raw(" "),
    ]);
    let included = view.included();
    let summary = if view.matches.is_empty() {
        String::new()
    } else {
        let total = format!(
            "{}{} match{} in {} file{}",
            view.matches.len(),
            if view.truncated { "+" } else { "" },
            if view.matches.len() == 1 { "" } else { "es" },
            SearchView::file_count(&view.matches),
            if SearchView::file_count(&view.matches) == 1 {
                ""
            } else {
                "s"
            },
        );
        if view.replacement().is_empty() {
            total
        } else {
            format!("{total} · {} to replace", included.len())
        }
    };

    for (index, (label, text, focus)) in [
        ("Find", &view.find, Focus::Find),
        ("Replace", &view.replace, Focus::Replace),
    ]
    .into_iter()
    .enumerate()
    {
        let right = if index == 0 {
            chips.clone()
        } else {
            Line::styled(format!("{summary} "), theme.fg(Role::Muted))
        };
        let right_width = right.width() as u16;
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(8), Constraint::Length(right_width)])
            .split(lines[index]);
        let focused = view.focus == focus;
        let label_span = if focused {
            Span::styled(
                format!("▸ {label:<width$}", width = LABEL_WIDTH - 2),
                theme.bold(Role::Cursor),
            )
        } else {
            Span::styled(
                format!("  {label:<width$}", width = LABEL_WIDTH - 2),
                theme.fg(Role::Muted),
            )
        };
        let room = usize::from(columns[0].width).saturating_sub(LABEL_WIDTH + 1);
        let (shown, cursor_x) = visible_text(text, room);
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                label_span,
                Span::styled(shown, theme.bold(Role::Ink)),
            ])),
            columns[0],
        );
        frame.render_widget(Paragraph::new(right), columns[1]);
        if focused {
            frame.set_cursor_position(Position::new(
                columns[0].x + LABEL_WIDTH as u16 + cursor_x as u16,
                columns[0].y,
            ));
        }
    }
}

/// The part of the text that fits in `room` columns with the cursor in view, and the
/// cursor's column within it.
fn visible_text(text: &TextArea, room: usize) -> (String, usize) {
    let (_, column) = text.cursor();
    let line = text.lines().first().cloned().unwrap_or_default();
    let start = (column + 1).saturating_sub(room);
    (
        line.chars().skip(start).take(room).collect(),
        column - start,
    )
}

fn draw_body(frame: &mut Frame<'_>, view: &SearchView, theme: &Theme, area: Rect) {
    if area.width < SPLIT_MIN_WIDTH {
        let block = pane_block(theme, false);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        draw_results(frame, view, theme, inner);
        return;
    }
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Fill(3), Constraint::Fill(2)])
        .split(area);
    for (index, draw) in [draw_results, draw_preview].into_iter().enumerate() {
        let block = pane_block(theme, index == 1);
        let inner = block.inner(columns[index]);
        frame.render_widget(block, columns[index]);
        draw(frame, view, theme, inner);
    }
}

fn draw_results(frame: &mut Frame<'_>, view: &SearchView, theme: &Theme, area: Rect) {
    let width = usize::from(area.width);
    let focused = view.focus == Focus::Results;
    let heading = |text: &str| {
        if focused {
            ruled_heading(theme, &format!("▸ {text}"), width, Role::Cursor)
        } else {
            ruled_heading(theme, text, width, Role::Muted)
        }
    };
    let query = view.query();
    let message = if let Some(error) = &view.search_error {
        Some((error.clone(), Role::Del))
    } else if query.chars().count() < super::MIN_QUERY {
        Some((
            "Type at least two characters to search every file in this project.".to_owned(),
            Role::Muted,
        ))
    } else if let Err(error) = &view.matcher {
        Some((error.clone(), Role::Del))
    } else if view.matches.is_empty() && (view.searching || view.answered.is_none()) {
        Some(("Searching…".to_owned(), Role::Muted))
    } else if view.matches.is_empty() {
        Some((
            format!("No matches for “{query}”. Ignored and binary files are not searched."),
            Role::Muted,
        ))
    } else {
        None
    };
    if let Some((text, role)) = message {
        frame.render_widget(
            Paragraph::new(vec![
                heading("Results"),
                Line::raw(""),
                Line::styled(text, theme.fg(role)),
            ])
            .wrap(Wrap { trim: true }),
            area,
        );
        return;
    }

    let parts = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(area);
    let files = SearchView::file_count(&view.matches);
    frame.render_widget(
        Paragraph::new(heading(&format!(
            "Results · {}{} in {files} file{}",
            view.matches.len(),
            if view.truncated { "+" } else { "" },
            if files == 1 { "" } else { "s" },
        ))),
        parts[0],
    );

    let matcher = view.matcher.as_ref().ok();
    let with_boxes = !view.replacement().is_empty();
    let mut items = Vec::new();
    let mut selected_row = 0;
    let mut current_file: Option<&str> = None;
    for (index, found) in view.matches.iter().enumerate() {
        if current_file != Some(found.path.as_str()) {
            current_file = Some(found.path.as_str());
            let count = view
                .matches
                .iter()
                .filter(|other| other.path == found.path)
                .count();
            items.push(ListItem::new(Line::from(vec![
                Span::styled(found.path.clone(), theme.bold(Role::Ink)),
                Span::styled(format!("  {count}"), theme.fg(Role::Muted)),
            ])));
        }
        if index == view.selected {
            selected_row = items.len();
        }
        items.push(ListItem::new(match_line(
            theme,
            found,
            matcher,
            width,
            index == view.selected,
            with_boxes.then(|| view.is_included(found)),
        )));
    }
    let mut state = ListState::default().with_selected(Some(selected_row));
    frame.render_stateful_widget(List::new(items), parts[1], &mut state);
}

/// `> [x]   42  text with the match highlighted`
fn match_line(
    theme: &Theme,
    found: &Match,
    matcher: Option<&Regex>,
    width: usize,
    selected: bool,
    included: Option<bool>,
) -> Line<'static> {
    let mut spans = vec![if selected {
        Span::styled("> ", theme.fg(Role::Cursor))
    } else {
        Span::raw("  ")
    }];
    if let Some(included) = included {
        spans.push(Span::styled(
            if included { "[x] " } else { "[ ] " },
            theme.fg(if included { Role::Add } else { Role::Muted }),
        ));
    }
    spans.push(Span::styled(
        format!("{:>5}  ", found.line),
        theme.fg(Role::Muted),
    ));
    let used: usize = spans.iter().map(Span::width).sum();
    let room = width.saturating_sub(used);

    let indent = found.text.len() - found.text.trim_start().len();
    let text = &found.text[indent..];
    let spans_found = matcher.map_or_else(Vec::new, |matcher| find_spans(matcher, text));
    spans.extend(highlighted(theme, text, &spans_found, room));
    let line = Line::from(spans);
    if selected {
        line.style(theme.bg(Role::Selection))
    } else {
        line
    }
}

/// `text` cut to `room` characters with the first match kept in view, matches emphasised.
fn highlighted(
    theme: &Theme,
    text: &str,
    matches: &[Range<usize>],
    room: usize,
) -> Vec<Span<'static>> {
    let characters: Vec<(usize, char)> = text.char_indices().collect();
    let first_match = matches
        .first()
        .and_then(|range| characters.iter().position(|(byte, _)| *byte == range.start))
        .unwrap_or(0);
    // Scroll right only when the match would otherwise fall off the end.
    let start = if first_match + 1 > room.saturating_sub(1) {
        first_match.saturating_sub(LEAD)
    } else {
        0
    };
    let end = (start + room).min(characters.len());
    let mut spans = Vec::new();
    if start > 0 {
        spans.push(Span::styled("…", theme.fg(Role::Muted)));
    }
    let emphasis = theme
        .fg(Role::Forge)
        .add_modifier(Modifier::BOLD | Modifier::UNDERLINED);
    let mut cursor = start;
    let byte_of = |index: usize| characters.get(index).map_or(text.len(), |(byte, _)| *byte);
    let char_of = |byte: usize| {
        characters
            .iter()
            .position(|(b, _)| *b >= byte)
            .unwrap_or(characters.len())
    };
    for range in matches {
        let (from, to) = (
            char_of(range.start).max(cursor),
            char_of(range.end).min(end),
        );
        if from >= to {
            continue;
        }
        if from > cursor {
            spans.push(Span::raw(text[byte_of(cursor)..byte_of(from)].to_owned()));
        }
        spans.push(Span::styled(
            text[byte_of(from)..byte_of(to)].to_owned(),
            emphasis,
        ));
        cursor = to;
    }
    if cursor < end {
        spans.push(Span::raw(text[byte_of(cursor)..byte_of(end)].to_owned()));
    }
    spans
}

fn draw_preview(frame: &mut Frame<'_>, view: &SearchView, theme: &Theme, area: Rect) {
    let width = usize::from(area.width);
    let Some(found) = view.selected_match() else {
        frame.render_widget(
            Paragraph::new(ruled_heading(theme, "Preview", width, Role::Muted)),
            area,
        );
        return;
    };
    // File names are case-sensitive, so this heading keeps the path as written.
    let title = fit(
        &format!("{}:{}", found.path, found.line),
        width.saturating_sub(2),
    );
    let title = title.trim_end().to_owned();
    let rule = width.saturating_sub(title.chars().count() + 1);
    let mut lines = vec![
        Line::from(vec![
            Span::styled(title, theme.bold(Role::Muted)),
            Span::raw(" "),
            Span::styled("─".repeat(rule), theme.fg(Role::Line)),
        ]),
        Line::raw(""),
    ];
    match view
        .context
        .as_ref()
        .filter(|(path, line, _)| *path == found.path && *line == found.line)
    {
        None => lines.push(Line::styled("Reading file…", theme.fg(Role::Muted))),
        Some((_, _, Err(error))) => lines.push(Line::styled(
            format!("Could not show this file. {error}"),
            theme.fg(Role::Del),
        )),
        Some((_, _, Ok(context))) => {
            let matcher = view.matcher.as_ref().ok();
            let replacement = view.replacement();
            let room = width.saturating_sub(7);
            for (offset, text) in context.lines.iter().enumerate() {
                let number = context.first_line + offset;
                let gutter = |sign: char, role: Role| {
                    Span::styled(format!("{sign}{number:>4}  "), theme.fg(role))
                };
                if number != found.line {
                    lines.push(Line::from(vec![
                        gutter(' ', Role::Muted),
                        Span::styled(
                            fit(text, room).trim_end().to_owned(),
                            theme.fg(Role::Subtle),
                        ),
                    ]));
                    continue;
                }
                let spans_found = matcher.map_or_else(Vec::new, |m| find_spans(m, text));
                let mut old = if replacement.is_empty() {
                    vec![gutter('▌', Role::Cursor)]
                } else {
                    vec![gutter('-', Role::Del)]
                };
                old.extend(highlighted(theme, text, &spans_found, room));
                lines.push(Line::from(old));
                if let (false, Some(matcher)) = (replacement.is_empty(), matcher) {
                    let new = replace_in_line(matcher, text, &replacement, &view.options);
                    lines.push(Line::from(vec![
                        gutter('+', Role::Add),
                        Span::styled(fit(&new, room).trim_end().to_owned(), theme.fg(Role::Add)),
                    ]));
                }
            }
        }
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
}

fn draw_confirm(
    frame: &mut Frame<'_>,
    view: &SearchView,
    theme: &Theme,
    confirm: &Confirm,
    area: Rect,
) {
    let count = confirm.targets.len();
    let mut lines = vec![
        Line::styled(
            format!(
                "Replace in {count} line{} across {} file{}?",
                if count == 1 { "" } else { "s" },
                confirm.files,
                if confirm.files == 1 { "" } else { "s" },
            ),
            theme.bold(Role::Ink),
        ),
        Line::from(vec![
            Span::styled("“", theme.fg(Role::Muted)),
            Span::styled(view.query(), theme.fg(Role::Del)),
            Span::styled("” → “", theme.fg(Role::Muted)),
            Span::styled(view.replacement(), theme.fg(Role::Add)),
            Span::styled("”", theme.fg(Role::Muted)),
        ]),
        Line::raw(""),
    ];
    for (place, old, new) in &confirm.preview {
        lines.push(Line::styled(place.clone(), theme.fg(Role::Muted)));
        lines.push(Line::styled(format!("- {old}"), theme.fg(Role::Del)));
        lines.push(Line::styled(format!("+ {new}"), theme.fg(Role::Add)));
    }
    if count > confirm.preview.len() {
        lines.push(Line::styled(
            format!("… and {} more", count - confirm.preview.len()),
            theme.fg(Role::Muted),
        ));
    }
    lines.push(Line::raw(""));
    lines.push(Line::styled(
        "Each file is saved under refs/forge/snapshots/ first, so you can undo it.",
        theme.fg(Role::Muted),
    ));
    draw_dialog(
        frame,
        theme,
        area,
        "Replace",
        Role::Del,
        lines,
        &[primary("y", "Replace"), hint("esc", "Cancel")],
    );
}
