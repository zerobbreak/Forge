use super::CommitsView;
use crate::{
    git::log::{CommitDetail, CommitInfo, LogRow, LogScope, RefLabel},
    theme::{Role, Theme},
    ui::{
        Hint, draw_bar, draw_title_bar, field, fit, format_utc, hint, key_line, pane_block,
        relative_age, repo_context, ruled_heading, status_line, unix_now, window,
    },
};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    text::{Line, Span},
    widgets::{List, ListItem, ListState, Paragraph, Wrap},
};

/// At this width the list and the detail sit side by side; below it one replaces the other.
const SPLIT_MIN_WIDTH: u16 = 100;
/// The author column appears from this list width.
const AUTHOR_MIN_WIDTH: usize = 64;
const AUTHOR_WIDTH: usize = 12;
const AGE_WIDTH: usize = 4;
/// Longest graph drawn; deeper lanes are cut off.
const GRAPH_MAX: usize = 16;
const HASH_WIDTH: usize = 7;
/// Lane colours, cycled by column.
const LANES: [Role; 5] = [Role::Cursor, Role::Forge, Role::Add, Role::Mod, Role::Del];

pub fn draw_commits(frame: &mut Frame<'_>, view: &CommitsView, theme: &Theme, user: Option<&str>) {
    let outer = window(theme);
    let inner = outer.inner(frame.area());
    frame.render_widget(outer, frame.area());

    let busy = (view.loading && !view.rows.is_empty()).then_some("reading history…");
    let notice = status_line(theme, busy, None);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(3),
            Constraint::Length(u16::from(notice.is_some())),
            Constraint::Length(1),
        ])
        .split(inner);
    let context = repo_context(theme, &view.path, view.tracking.as_ref());
    draw_title_bar(frame, theme, rows[0], context, user);
    draw_body(frame, view, theme, rows[1]);
    if let Some(notice) = notice {
        frame.render_widget(Paragraph::new(notice), rows[2]);
    }
    let width = usize::from(rows[3].width);
    draw_bar(frame, theme, rows[3], key_line(theme, &hints(view), width));
}

fn hints(view: &CommitsView) -> Vec<Hint> {
    vec![
        hint("j/k", "Move"),
        hint("enter", "Details"),
        hint(
            "a",
            match view.scope {
                LogScope::All => "Current branch",
                LogScope::Head => "All branches",
            },
        ),
        hint("esc", if view.detail_open { "List" } else { "Changes" }),
        hint("r", "Refresh"),
        hint("q", "Quit"),
    ]
}

fn draw_body(frame: &mut Frame<'_>, view: &CommitsView, theme: &Theme, area: Rect) {
    if area.width < SPLIT_MIN_WIDTH {
        let block = pane_block(theme, false);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        if view.detail_open {
            draw_detail(frame, view, theme, inner);
        } else {
            draw_list(frame, view, theme, inner);
        }
        return;
    }
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Fill(3), Constraint::Fill(2)])
        .split(area);
    for (index, draw) in [draw_list, draw_detail].into_iter().enumerate() {
        let block = pane_block(theme, index == 1);
        let inner = block.inner(columns[index]);
        frame.render_widget(block, columns[index]);
        draw(frame, view, theme, inner);
    }
}

fn draw_list(frame: &mut Frame<'_>, view: &CommitsView, theme: &Theme, area: Rect) {
    let width = usize::from(area.width);
    let scope = match view.scope {
        LogScope::All => "all branches",
        LogScope::Head => "current branch",
    };
    let count = view.commit_count();
    let title = format!(
        "Commits · {scope} · {count}{}",
        if view.more { "+" } else { "" }
    );
    let parts = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(area);
    frame.render_widget(
        Paragraph::new(ruled_heading(theme, &title, width, Role::Cursor)),
        parts[0],
    );

    let message = if let Some(error) = &view.load_error {
        Some((
            format!("Could not read the history. {error} Press r to retry."),
            Role::Del,
        ))
    } else if view.rows.is_empty() && view.loading {
        Some(("Reading history…".to_owned(), Role::Muted))
    } else if view.rows.is_empty() {
        Some((
            "No commits yet. Stage a file in Changes, then press c to make the first one."
                .to_owned(),
            Role::Muted,
        ))
    } else {
        None
    };
    if let Some((text, role)) = message {
        frame.render_widget(
            Paragraph::new(Line::styled(text, theme.fg(role))).wrap(Wrap { trim: true }),
            parts[1],
        );
        return;
    }

    let graph_width = view
        .rows
        .iter()
        .map(|row| row.graph.trim_end().chars().count())
        .max()
        .unwrap_or(0)
        .min(GRAPH_MAX);
    let now = unix_now();
    let items: Vec<ListItem> = view
        .rows
        .iter()
        .enumerate()
        .map(|(index, row)| {
            ListItem::new(row_line(
                theme,
                row,
                graph_width,
                width,
                index == view.selected,
                now,
            ))
        })
        .collect();
    let mut state = ListState::default().with_selected(Some(view.selected));
    frame.render_stateful_widget(List::new(items), parts[1], &mut state);
}

/// `> graph  hash  [refs] subject          author  age`
fn row_line(
    theme: &Theme,
    row: &LogRow,
    graph_width: usize,
    width: usize,
    selected: bool,
    now: u64,
) -> Line<'static> {
    let mut spans = vec![if selected {
        Span::styled("> ", theme.fg(Role::Cursor))
    } else {
        Span::raw("  ")
    }];
    spans.extend(graph_spans(theme, &row.graph, graph_width));
    let Some(commit) = &row.commit else {
        return Line::from(spans);
    };

    let show_author = width >= AUTHOR_MIN_WIDTH;
    let right = AGE_WIDTH + 1 + if show_author { AUTHOR_WIDTH + 1 } else { 0 };
    let fixed = 2 + graph_width + 1 + HASH_WIDTH + 1;
    let room = width.saturating_sub(fixed + right);
    spans.push(Span::raw(" "));
    spans.push(Span::styled(
        fit(&commit.short, HASH_WIDTH),
        theme.fg(Role::Muted),
    ));
    spans.push(Span::raw(" "));

    // Ref chips may take up to 60% of the room, always leaving the subject some.
    let (chips, chips_width) = chip_spans(theme, commit, room * 3 / 5);
    spans.extend(chips);
    let subject_width = room.saturating_sub(chips_width);
    spans.push(Span::styled(
        fit(&commit.subject, subject_width),
        theme.fg(Role::Ink),
    ));
    if show_author {
        spans.push(Span::styled(
            format!(" {}", fit(&commit.author, AUTHOR_WIDTH)),
            theme.fg(Role::Muted),
        ));
    }
    spans.push(Span::styled(
        format!(" {:>AGE_WIDTH$}", relative_age(now, commit.time)),
        theme.fg(Role::Muted),
    ));

    let line = Line::from(spans);
    if selected {
        line.style(theme.bg(Role::Selection))
    } else {
        line
    }
}

/// Bracketed ref names that fit in `budget` columns, plus the columns used.
fn chip_spans(theme: &Theme, commit: &CommitInfo, budget: usize) -> (Vec<Span<'static>>, usize) {
    let mut spans = Vec::new();
    let mut used = 0;
    for label in &commit.refs {
        let text = chip_text(label);
        let cost = text.chars().count() + 1;
        if used + cost > budget {
            break;
        }
        used += cost;
        spans.push(Span::styled(text, chip_style(theme, label)));
        spans.push(Span::raw(" "));
    }
    (spans, used)
}

fn chip_text(label: &RefLabel) -> String {
    match label {
        RefLabel::Head(name) if name == "HEAD" => "[HEAD]".to_owned(),
        RefLabel::Head(name) => format!("[⎇ {name}]"),
        RefLabel::Branch(name) | RefLabel::Remote(name) => format!("[{name}]"),
        RefLabel::Tag(name) => format!("[tag: {name}]"),
    }
}

fn chip_style(theme: &Theme, label: &RefLabel) -> ratatui::style::Style {
    match label {
        RefLabel::Head(_) => theme.bold(Role::Forge),
        RefLabel::Branch(_) => theme.fg(Role::Cursor),
        RefLabel::Remote(_) => theme.fg(Role::Muted),
        RefLabel::Tag(_) => theme.fg(Role::Mod),
    }
}

/// Git's ASCII lanes redrawn with box characters, one colour per lane, padded to `width`.
fn graph_spans(theme: &Theme, graph: &str, width: usize) -> Vec<Span<'static>> {
    let mut spans: Vec<Span<'static>> = graph
        .trim_end()
        .chars()
        .take(width)
        .enumerate()
        .map(|(column, symbol)| {
            let glyph = match symbol {
                '*' => '●',
                '|' => '│',
                '_' => '─',
                other => other,
            };
            Span::styled(
                glyph.to_string(),
                theme.fg(LANES[(column / 2) % LANES.len()]),
            )
        })
        .collect();
    let drawn = spans.len();
    if drawn < width {
        spans.push(Span::raw(" ".repeat(width - drawn)));
    }
    spans
}

fn draw_detail(frame: &mut Frame<'_>, view: &CommitsView, theme: &Theme, area: Rect) {
    let width = usize::from(area.width);
    let Some(commit) = view.selected_commit() else {
        frame.render_widget(
            Paragraph::new(vec![
                ruled_heading(theme, "Commit", width, Role::Muted),
                Line::styled("Nothing selected.", theme.fg(Role::Muted)),
            ]),
            area,
        );
        return;
    };
    let mut lines = vec![
        ruled_heading(
            theme,
            &format!("Commit {}", commit.short),
            width,
            Role::Muted,
        ),
        Line::raw(""),
    ];
    match view
        .detail
        .as_ref()
        .filter(|(hash, _)| *hash == commit.hash)
    {
        Some((_, Ok(detail))) => lines.extend(detail_lines(theme, commit, detail, width)),
        Some((_, Err(error))) => {
            lines.push(Line::styled(commit.subject.clone(), theme.bold(Role::Ink)));
            lines.push(Line::raw(""));
            lines.push(Line::styled(
                format!("Could not read this commit. {error} Press r to retry."),
                theme.fg(Role::Del),
            ));
        }
        None => {
            lines.push(Line::styled(commit.subject.clone(), theme.bold(Role::Ink)));
            lines.push(Line::raw(""));
            lines.push(Line::styled("Reading commit…", theme.fg(Role::Muted)));
        }
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
}

fn detail_lines(
    theme: &Theme,
    commit: &CommitInfo,
    detail: &CommitDetail,
    width: usize,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let (subject, body) = detail
        .message
        .split_once('\n')
        .unwrap_or((&detail.message, ""));
    lines.push(Line::styled(subject.to_owned(), theme.bold(Role::Ink)));
    lines.push(Line::raw(""));
    lines.push(field(theme, "commit", detail.hash.clone()));
    lines.push(field(
        theme,
        "author",
        format!("{} <{}>", detail.author, detail.email),
    ));
    lines.push(field(theme, "date", {
        let age = relative_age(unix_now(), detail.time);
        let age = if age == "now" {
            "just now".to_owned()
        } else {
            format!("{age} ago")
        };
        format!("{} · {age}", format_utc(detail.time))
    }));
    if !detail.parents.is_empty() {
        let parents: Vec<_> = detail
            .parents
            .iter()
            .map(|parent| parent.chars().take(HASH_WIDTH).collect::<String>())
            .collect();
        let merge = if parents.len() > 1 { " (merge)" } else { "" };
        lines.push(field(
            theme,
            "parents",
            format!("{}{merge}", parents.join(", ")),
        ));
    }
    if !commit.refs.is_empty() {
        let mut spans = vec![Span::styled(
            format!("{:<10}", "refs"),
            theme.fg(Role::Muted),
        )];
        for label in &commit.refs {
            spans.push(Span::styled(chip_text(label), chip_style(theme, label)));
            spans.push(Span::raw(" "));
        }
        lines.push(Line::from(spans));
    }

    let body = body.trim();
    if !body.is_empty() {
        lines.push(Line::raw(""));
        lines.extend(body.lines().map(|line| Line::raw(line.to_owned())));
    }

    lines.push(Line::raw(""));
    let added: usize = detail.files.iter().filter_map(|file| file.added).sum();
    let removed: usize = detail.files.iter().filter_map(|file| file.removed).sum();
    let merge = detail.parents.len() > 1;
    lines.push(ruled_heading(
        theme,
        &format!(
            "{} · {} file{}  +{added} −{removed}",
            if merge { "Brought in" } else { "Files" },
            detail.files.len(),
            if detail.files.len() == 1 { "" } else { "s" },
        ),
        width,
        Role::Muted,
    ));
    for file in &detail.files {
        let counts = match (file.added, file.removed) {
            (Some(added), Some(removed)) => vec![
                Span::styled(format!("+{added}"), theme.fg(Role::Add)),
                Span::raw(" "),
                Span::styled(format!("−{removed}"), theme.fg(Role::Del)),
            ],
            _ => vec![Span::styled("bin", theme.fg(Role::Muted))],
        };
        let used: usize = counts.iter().map(Span::width).sum();
        let mut spans = counts;
        spans.push(Span::raw(" ".repeat(10usize.saturating_sub(used).max(1))));
        spans.push(Span::raw(file.path.clone()));
        lines.push(Line::from(spans));
    }
    lines
}
