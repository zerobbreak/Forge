use super::{ChangesView, Discard, Pane};
use crate::{
    git::changes::{FileChange, FileDiff, LineKind, SNAPSHOT_NAMESPACE},
    screens::Screen,
    theme::{Role, Theme},
    ui::{
        draw_bar, draw_tabs, draw_title_bar, fit, heading, hint, key_line, popup, primary,
        relative_age, repo_context, ruled_heading, status_line, unix_now, window,
    },
};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{
        Block, BorderType, Borders, Clear, List, ListItem, ListState, Padding, Paragraph, Wrap,
    },
};

/// Width of the repository navigation column, as in the design's 176px sidebar.
const NAV_WIDTH: u16 = 22;
/// Below this width the navigation column is hidden.
const NAV_MIN_WIDTH: u16 = 100;
/// Below this width only the focused pane is shown.
const SPLIT_MIN_WIDTH: u16 = 60;
/// Diff lines previewed in the discard dialog.
const PREVIEW_LINES: usize = 6;

pub fn draw_changes(frame: &mut Frame<'_>, view: &ChangesView, theme: &Theme, user: Option<&str>) {
    let outer = window(theme);
    let inner = outer.inner(frame.area());
    frame.render_widget(outer, frame.area());

    let notice = status_line(theme, view.busy, view.notice.as_ref());
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(3),
            Constraint::Length(u16::from(notice.is_some())),
            Constraint::Length(1),
        ])
        .split(inner);
    let tracking = view.tree.as_ref().map(|tree| &tree.tracking);
    let context = repo_context(theme, &view.path, tracking);
    draw_title_bar(frame, theme, rows[0], context, user);
    draw_tabs(frame, theme, rows[1], Screen::Changes);
    draw_body(frame, view, theme, rows[2]);
    if let Some(notice) = notice {
        frame.render_widget(Paragraph::new(notice), rows[3]);
    }
    let width = usize::from(rows[4].width);
    draw_bar(frame, theme, rows[4], key_line(theme, &hints(view), width));

    if let Some(discard) = &view.confirm {
        draw_confirm(frame, theme, discard, inner);
    }
}

fn hints(view: &ChangesView) -> Vec<crate::ui::Hint> {
    let staged = view.selected_change().is_some_and(|change| change.staged);
    let mut hints = if staged {
        vec![primary("u", "Unstage")]
    } else {
        vec![
            primary("s", "Stage hunk"),
            hint("a", "Stage file"),
            hint("x", "Discard"),
        ]
    };
    hints.extend([
        hint("c", "Commit"),
        hint("tab", "Next pane"),
        hint("/", "Commands"),
        hint("r", "Refresh"),
        hint("esc", "Projects"),
        hint("q", "Quit"),
    ]);
    hints
}

fn draw_body(frame: &mut Frame<'_>, view: &ChangesView, theme: &Theme, area: Rect) {
    view.nav_visible.set(area.width >= NAV_MIN_WIDTH);
    if area.width < SPLIT_MIN_WIDTH {
        let pane = pane_block(theme, false).inner(area);
        match view.focus {
            Pane::Files | Pane::Nav => draw_files(frame, view, theme, pane),
            Pane::Diff => draw_diff(frame, view, theme, pane),
        }
        return;
    }
    let mut constraints = vec![Constraint::Fill(10), Constraint::Fill(14)];
    let with_nav = area.width >= NAV_MIN_WIDTH;
    if with_nav {
        constraints.insert(0, Constraint::Length(NAV_WIDTH));
    }
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(constraints)
        .split(area);
    let (files, diff) = if with_nav {
        let block = pane_block(theme, false);
        let nav = block.inner(columns[0]);
        frame.render_widget(block, columns[0]);
        draw_nav(frame, view, theme, nav);
        (columns[1], columns[2])
    } else {
        (columns[0], columns[1])
    };
    let files_block = pane_block(theme, with_nav);
    let files_area = files_block.inner(files);
    frame.render_widget(files_block, files);
    draw_files(frame, view, theme, files_area);
    let diff_block = pane_block(theme, true);
    let diff_area = diff_block.inner(diff);
    frame.render_widget(diff_block, diff);
    draw_diff(frame, view, theme, diff_area);
}

/// A pane with a rule on its left edge when it follows another pane.
fn pane_block(theme: &Theme, divided: bool) -> Block<'static> {
    Block::default()
        .borders(if divided {
            Borders::LEFT
        } else {
            Borders::NONE
        })
        .border_style(theme.fg(Role::Line))
        .padding(Padding::new(1, 1, 1, 0))
}

/// Section heading for a pane; the focused pane's headings are marked and highlighted.
fn pane_heading(theme: &Theme, text: &str, width: usize, focused: bool) -> Line<'static> {
    if focused {
        ruled_heading(theme, &format!("▸ {text}"), width, Role::Cursor)
    } else {
        ruled_heading(theme, text, width, Role::Muted)
    }
}

fn draw_nav(frame: &mut Frame<'_>, view: &ChangesView, theme: &Theme, area: Rect) {
    let width = usize::from(area.width);
    let focused = view.focus == Pane::Nav;
    let changed = view.tree.as_ref().map(|tree| tree.changed_files());
    let branches = view.tree.as_ref().map(|tree| tree.branches);
    // (number key, label, count); the first entry is the screen being shown.
    let entries = [
        ("1", "Changes", changed),
        ("2", "Commits", None),
        ("3", "Branches", branches),
        ("4", "Pull Requests", None),
        ("5", "Search", None),
        ("6", "Remote", None),
    ];
    let mut lines = vec![
        pane_heading(theme, "Repository", width, focused),
        Line::raw(""),
    ];
    for (index, (key, label, count)) in entries.into_iter().enumerate() {
        let count = count.map(|count| count.to_string()).unwrap_or_default();
        let label_width = width.saturating_sub(4 + count.chars().count());
        let text = format!("{key} {}{count}", fit(label, label_width));
        let cursor = focused && index == view.nav_cursor;
        let current = index == 0;
        lines.push(if cursor || (current && !focused) {
            Line::from(vec![
                Span::styled("> ", theme.fg(Role::Cursor)),
                Span::styled(text, theme.bold(Role::Cursor)),
            ])
            .style(theme.bg(Role::Selection))
        } else if current {
            Line::styled(format!("• {text}"), theme.bold(Role::Ink))
        } else {
            Line::styled(format!("  {text}"), theme.fg(Role::Muted))
        });
    }
    frame.render_widget(Paragraph::new(lines), area);
}

fn draw_files(frame: &mut Frame<'_>, view: &ChangesView, theme: &Theme, area: Rect) {
    let width = usize::from(area.width);
    let focused = view.focus == Pane::Files;
    let Some(tree) = &view.tree else {
        let message = match &view.load_error {
            Some(error) => Line::styled(
                format!("Could not read this repository. {error} Press r to retry."),
                theme.fg(Role::Del),
            ),
            None => Line::styled("Reading working tree…", theme.fg(Role::Muted)),
        };
        frame.render_widget(
            Paragraph::new(vec![
                pane_heading(theme, "Changes", width, focused),
                message,
            ])
            .wrap(Wrap { trim: true }),
            area,
        );
        return;
    };

    let with_summary = area.height >= 12;
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(if with_summary { 4 } else { 0 }),
        ])
        .split(area);

    if tree.staged.is_empty() && tree.unstaged.is_empty() {
        frame.render_widget(
            Paragraph::new(vec![
                pane_heading(theme, "Changes", width, focused),
                Line::raw(""),
                Line::styled("Working tree clean.", theme.fg(Role::Add)),
                Line::styled("Nothing to stage or commit.", theme.fg(Role::Muted)),
            ]),
            sections[0],
        );
    } else {
        let mut rows = vec![ListItem::new(pane_heading(
            theme,
            &format!("Staged · {}", tree.staged.len()),
            width,
            focused,
        ))];
        let mut selected_row = None;
        let mut index = 0;
        for (section, changes) in [(None, &tree.staged), (Some("Unstaged"), &tree.unstaged)] {
            if let Some(section) = section {
                rows.push(ListItem::new(""));
                rows.push(ListItem::new(pane_heading(
                    theme,
                    &format!("{section} · {}", changes.len()),
                    width,
                    focused,
                )));
            }
            for change in changes {
                let is_selected = index == view.selected;
                if is_selected {
                    selected_row = Some(rows.len());
                }
                rows.push(ListItem::new(file_row(theme, change, is_selected, width)));
                index += 1;
            }
        }
        let mut state = ListState::default();
        state.select(selected_row);
        frame.render_stateful_widget(
            List::new(rows).highlight_style(theme.bg(Role::Selection)),
            sections[0],
            &mut state,
        );
    }

    if with_summary {
        let (added, removed) =
            tree.staged
                .iter()
                .chain(&tree.unstaged)
                .fold((0, 0), |(added, removed), change| {
                    (
                        added + change.added.unwrap_or(0),
                        removed + change.removed.unwrap_or(0),
                    )
                });
        let files = tree.changed_files();
        let fetched = tree.last_fetch.map_or_else(
            || "never fetched".to_owned(),
            |then| format!("last fetch {} ago", relative_age(unix_now(), then)),
        );
        let lines = vec![
            Line::styled("─".repeat(width), theme.fg(Role::Line)),
            heading(theme, "Summary"),
            Line::from(vec![
                Span::styled(
                    format!("{files} {} · ", if files == 1 { "file" } else { "files" }),
                    theme.fg(Role::Muted),
                ),
                Span::styled(format!("+{added}"), theme.fg(Role::Add)),
                Span::raw(" "),
                Span::styled(format!("−{removed}"), theme.fg(Role::Del)),
            ]),
            Line::styled(fetched, theme.fg(Role::Subtle)),
        ];
        frame.render_widget(Paragraph::new(lines), sections[1]);
    }
}

/// `> M src/services/cache.py   +6 −2`, with the directory dimmed.
pub fn file_row(theme: &Theme, change: &FileChange, selected: bool, width: usize) -> Line<'static> {
    let stat = match (change.added, change.removed) {
        (None, _) | (_, None) => "bin".to_owned(),
        (Some(added), Some(removed)) => [
            (added > 0).then(|| format!("+{added}")),
            (removed > 0).then(|| format!("−{removed}")),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" "),
    };
    let stat_width = stat.chars().count();
    let path_width = width.saturating_sub(4 + stat_width + usize::from(stat_width > 0));
    let shown = tail_fit(&change.path, path_width);
    let (directory, name) = match shown.rfind('/') {
        Some(split) => (shown[..=split].to_owned(), shown[split + 1..].to_owned()),
        None => (String::new(), shown),
    };
    let padding = path_width.saturating_sub(directory.chars().count() + name.chars().count());
    let status_role = match change.state.symbol() {
        'M' | 'T' => Role::Mod,
        '+' => Role::Add,
        'D' | 'U' => Role::Del,
        'R' | 'C' => Role::Cursor,
        _ => Role::Muted,
    };
    Line::from(vec![
        Span::styled(if selected { "> " } else { "  " }, theme.fg(Role::Cursor)),
        Span::styled(
            format!("{} ", change.state.symbol()),
            theme.bold(status_role),
        ),
        Span::styled(directory, theme.fg(Role::Muted)),
        Span::styled(name, theme.fg(Role::Ink)),
        Span::raw(" ".repeat(padding + usize::from(stat_width > 0))),
        Span::styled(stat, theme.fg(Role::Muted)),
    ])
}

/// Keeps the end of a path (the file name) when it must be shortened: `…ices/cache.py`.
fn tail_fit(path: &str, width: usize) -> String {
    let length = path.chars().count();
    if length <= width {
        return path.to_owned();
    }
    if width == 0 {
        return String::new();
    }
    let tail: String = path.chars().skip(length - (width - 1)).collect();
    format!("…{tail}")
}

fn draw_diff(frame: &mut Frame<'_>, view: &ChangesView, theme: &Theme, area: Rect) {
    let width = usize::from(area.width);
    let focused = view.focus == Pane::Diff;
    let Some(change) = view.selected_change() else {
        frame.render_widget(
            Paragraph::new(vec![
                pane_heading(theme, "Diff", width, focused),
                Line::styled("Select a file to see its changes.", theme.fg(Role::Muted)),
            ]),
            area,
        );
        return;
    };
    let diff = view
        .diff
        .as_ref()
        .filter(|loaded| loaded.key == super::EntryKey::of(change));
    let side = if change.staged { " · staged" } else { "" };
    let title = match diff.map(|loaded| &loaded.result) {
        Some(Ok(diff)) if !diff.hunks.is_empty() => {
            format!(
                "Diff{side} · hunk {} of {}",
                view.hunk + 1,
                diff.hunks.len()
            )
        }
        _ => format!("Diff{side}"),
    };
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(1),
        ])
        .split(area);
    frame.render_widget(
        Paragraph::new(pane_heading(theme, &title, width, focused)),
        rows[0],
    );
    let file = match &change.original {
        Some(original) => format!("{original} → {}", change.path),
        None => change.path.clone(),
    };
    frame.render_widget(
        Paragraph::new(Line::styled(fit(&file, width), theme.bold(Role::Ink))),
        rows[1],
    );

    let (text, role) = match diff.map(|loaded| &loaded.result) {
        None => ("Loading diff…".to_owned(), Role::Muted),
        Some(Err(error)) => (
            format!("Could not read the diff. {error} Press r to refresh."),
            Role::Del,
        ),
        Some(Ok(diff)) if diff.binary => (
            "Binary file — no text diff. Press a to stage the whole file.".to_owned(),
            Role::Muted,
        ),
        Some(Ok(diff)) if diff.hunks.is_empty() => (
            "No text changes (empty file or mode change). Press a to stage it.".to_owned(),
            Role::Muted,
        ),
        Some(Ok(diff)) => return draw_hunks(frame, view, theme, diff, rows[2]),
    };
    frame.render_widget(
        Paragraph::new(vec![Line::raw(""), Line::styled(text, theme.fg(role))])
            .wrap(Wrap { trim: true }),
        rows[2],
    );
}

fn draw_hunks(
    frame: &mut Frame<'_>,
    view: &ChangesView,
    theme: &Theme,
    diff: &FileDiff,
    area: Rect,
) {
    let width = usize::from(area.width);
    let mut lines = vec![Line::raw("")];
    let mut selected_span = (0, 0);
    for (index, hunk) in diff.hunks.iter().enumerate() {
        let selected = index == view.hunk;
        if index > 0 {
            lines.push(Line::raw(""));
        }
        let start = lines.len();
        let gutter = || {
            if selected {
                Span::styled("▌", theme.fg(Role::Cursor))
            } else {
                Span::raw(" ")
            }
        };
        lines.push(Line::from(vec![
            gutter(),
            Span::styled(
                fit(&hunk.header, width.saturating_sub(1)),
                theme.fg(if selected { Role::Cursor } else { Role::Subtle }),
            ),
        ]));
        for line in &hunk.lines {
            let (sign, number, style) = match line.kind {
                LineKind::Added => ("+", line.new, line_style(theme, Role::Add, Role::AddBg)),
                LineKind::Removed => ("-", line.old, line_style(theme, Role::Del, Role::DelBg)),
                LineKind::Context => (" ", line.new, theme.fg(Role::Muted)),
                LineKind::Marker => ("", None, theme.fg(Role::Subtle)),
            };
            let number = number.map_or_else(|| "    ".to_owned(), |number| format!("{number:>4}"));
            let text = if line.kind == LineKind::Marker {
                line.text.clone()
            } else {
                format!("{number} {sign} {}", line.text.replace('\t', "    "))
            };
            lines.push(Line::from(vec![
                gutter(),
                Span::styled(fit(&text, width.saturating_sub(1)), style),
            ]));
        }
        if selected {
            selected_span = (start, lines.len());
        }
    }

    let height = usize::from(area.height);
    let (start, end) = selected_span;
    let base = if end <= height {
        0
    } else {
        start.saturating_sub(1)
    };
    let scroll = (base + usize::from(view.diff_scroll)).min(lines.len().saturating_sub(1));
    frame.render_widget(
        Paragraph::new(lines).scroll((u16::try_from(scroll).unwrap_or(u16::MAX), 0)),
        area,
    );
}

fn line_style(theme: &Theme, foreground: Role, background: Role) -> Style {
    theme.fg(foreground).patch(theme.bg(background))
}

fn draw_confirm(frame: &mut Frame<'_>, theme: &Theme, discard: &Discard, area: Rect) {
    let change = &discard.change;
    let untracked = change.state.symbol() == '?';
    let (question, added, removed) = match &discard.hunk {
        Some(hunk) => (
            format!(
                "Discard hunk {} of {} in {}?",
                hunk.index + 1,
                hunk.total,
                change.path
            ),
            hunk.added,
            hunk.removed,
        ),
        None if untracked => (
            format!("Delete untracked file {}?", change.path),
            change.added.unwrap_or(0),
            0,
        ),
        None => (
            format!("Discard all unstaged changes to {}?", change.path),
            change.added.unwrap_or(0),
            change.removed.unwrap_or(0),
        ),
    };

    let mut lines = vec![
        Line::styled(question, theme.bold(Role::Ink)),
        Line::from(vec![
            Span::styled("You will lose: ", theme.fg(Role::Muted)),
            Span::styled(format!("+{added}"), theme.fg(Role::Add)),
            Span::raw(" "),
            Span::styled(format!("−{removed}"), theme.fg(Role::Del)),
            Span::styled(" lines from your working copy", theme.fg(Role::Muted)),
        ]),
    ];
    if let Some(hunk) = &discard.hunk {
        lines.push(Line::raw(""));
        lines.push(Line::styled(hunk.header.clone(), theme.fg(Role::Subtle)));
        let changed = hunk
            .lines
            .iter()
            .filter(|line| matches!(line.kind, LineKind::Added | LineKind::Removed));
        for line in changed.take(PREVIEW_LINES) {
            let (sign, role) = if line.kind == LineKind::Added {
                ("+", Role::Add)
            } else {
                ("-", Role::Del)
            };
            lines.push(Line::styled(
                format!("{sign} {}", line.text.replace('\t', "    ")),
                theme.fg(role),
            ));
        }
    }
    lines.push(Line::raw(""));
    lines.push(Line::styled(
        format!("A recovery snapshot is saved under {SNAPSHOT_NAMESPACE}/ first."),
        theme.fg(Role::Muted),
    ));

    let height = u16::try_from(lines.len())
        .unwrap_or(u16::MAX)
        .saturating_add(3);
    let popup = popup(area, 68, height);
    frame.render_widget(Clear, popup);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme.fg(Role::Del))
        .title(Span::styled(" DISCARD CHANGES ", theme.bold(Role::Del)))
        .padding(Padding::horizontal(1))
        .style(theme.bg(Role::Panel).patch(theme.fg(Role::Ink)));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(inner);
    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }),
        sections[0],
    );
    let hints = [primary("y", "Discard"), hint("esc", "Keep")];
    let width = usize::from(sections[1].width);
    draw_bar(frame, theme, sections[1], key_line(theme, &hints, width));
}

#[cfg(test)]
mod tests {
    use super::tail_fit;

    #[test]
    fn long_paths_keep_their_file_name() {
        assert_eq!(
            tail_fit("src/services/cache.py", 30),
            "src/services/cache.py"
        );
        assert_eq!(tail_fit("src/services/cache.py", 12), "…es/cache.py");
        assert_eq!(tail_fit("abc", 0), "");
    }
}
