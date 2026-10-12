use super::{
    CommitView, Field, HookRun, Phase, SUBJECT_LIMIT, TYPES, Toggle, subject_type, trailers,
};
use crate::{
    editor::TextArea,
    screens::changes::file_row,
    theme::{Role, Theme},
    ui::{
        Hint, draw_bar, draw_title_bar, fit, heading, hint, key_line, primary, repo_context,
        ruled_heading, status_line, window,
    },
};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Position, Rect},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Padding, Paragraph},
};

/// Below this width the staged-files pane is hidden.
const SIDE_MIN_WIDTH: u16 = 70;
const SIDE_MAX_WIDTH: u16 = 40;

pub fn draw_commit(frame: &mut Frame<'_>, view: &CommitView, theme: &Theme, user: Option<&str>) {
    let outer = window(theme);
    let inner = outer.inner(frame.area());
    frame.render_widget(outer, frame.area());

    let busy = match view.phase {
        Phase::Editing => None,
        Phase::Committing if view.skip_hooks => Some("committing (hooks skipped)…"),
        Phase::Committing => Some("committing — running hooks…"),
        Phase::Pushing => Some("pushing…"),
    };
    let notice = status_line(theme, busy, view.notice.as_ref());
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

    let body = rows[1];
    if body.width >= SIDE_MIN_WIDTH {
        let side = (body.width * 2 / 5).min(SIDE_MAX_WIDTH);
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(side), Constraint::Min(20)])
            .split(body);
        let block = pane_block(theme, false);
        let area = block.inner(columns[0]);
        frame.render_widget(block, columns[0]);
        draw_side(frame, view, theme, area);
        let block = pane_block(theme, true);
        let area = block.inner(columns[1]);
        frame.render_widget(block, columns[1]);
        draw_message(frame, view, theme, area);
    } else {
        let block = pane_block(theme, false);
        let area = block.inner(body);
        frame.render_widget(block, body);
        draw_message(frame, view, theme, area);
    }

    if let Some(notice) = notice {
        frame.render_widget(Paragraph::new(notice), rows[2]);
    }
    let width = usize::from(rows[3].width);
    draw_bar(frame, theme, rows[3], key_line(theme, &hints(view), width));
}

fn hints(view: &CommitView) -> Vec<Hint> {
    let mut hints = vec![primary("ctrl+s", "Commit"), hint("tab", "Next field")];
    match view.focus {
        Field::Type => hints.push(hint("←→", "Type")),
        Field::Subject => hints.push(hint("enter", "Body")),
        Field::Body => hints.push(hint("enter", "New line")),
        Field::Options => {
            hints.push(hint("space", "Toggle"));
            hints.push(hint("e", "Amend"));
            hints.push(hint("p", "Push"));
        }
    }
    hints.push(hint("esc", "Cancel"));
    hints
}

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

fn draw_side(frame: &mut Frame<'_>, view: &CommitView, theme: &Theme, area: Rect) {
    let width = usize::from(area.width);
    let mut lines = vec![ruled_heading(
        theme,
        &format!("Staged · {}", view.staged.len()),
        width,
        Role::Muted,
    )];
    if view.staged.is_empty() {
        lines.push(Line::styled("Nothing staged.", theme.fg(Role::Muted)));
    }
    let staged_rows = usize::from(area.height / 3).max(3);
    for change in view.staged.iter().take(staged_rows) {
        lines.push(file_row(theme, change, false, width));
    }
    if view.staged.len() > staged_rows {
        lines.push(Line::styled(
            format!("  … {} more", view.staged.len() - staged_rows),
            theme.fg(Role::Subtle),
        ));
    }
    lines.push(Line::raw(""));
    lines.push(ruled_heading(theme, "Hooks", width, Role::Muted));
    match &view.info {
        None => lines.push(Line::styled("checking…", theme.fg(Role::Muted))),
        Some(Err(error)) => lines.push(Line::styled(
            format!("Could not read hooks: {error}"),
            theme.fg(Role::Del),
        )),
        Some(Ok(info)) if info.hooks.is_empty() => {
            lines.push(Line::styled(
                "No commit hooks installed.",
                theme.fg(Role::Muted),
            ));
        }
        Some(Ok(info)) => {
            let (glyph, state, role) = match (view.phase, view.skip_hooks, view.hooks) {
                (_, true, _) => ("–", "skipped", Role::Subtle),
                (Phase::Committing, false, _) => ("◐", "running", Role::Mod),
                (_, false, HookRun::Passed) => ("✓", "passed", Role::Add),
                (_, false, HookRun::Failed) => ("✕", "failed", Role::Del),
                (_, false, HookRun::NotRun) => ("·", "runs on commit", Role::Subtle),
            };
            for hook in &info.hooks {
                lines.push(Line::from(vec![
                    Span::styled(format!("{glyph} "), theme.fg(role)),
                    Span::styled(hook.clone(), theme.fg(Role::Ink)),
                    Span::styled(format!(" · {state}"), theme.fg(Role::Subtle)),
                ]));
            }
        }
    }
    let room = usize::from(area.height).saturating_sub(lines.len() + 1);
    if !view.output.is_empty() && room > 0 {
        lines.push(Line::raw(""));
        let start = view.output.len().saturating_sub(room);
        lines.extend(
            view.output[start..]
                .iter()
                .map(|line| Line::styled(fit(line, width), theme.fg(Role::Muted))),
        );
    }
    frame.render_widget(Paragraph::new(lines), area);
}

fn draw_message(frame: &mut Frame<'_>, view: &CommitView, theme: &Theme, area: Rect) {
    let width = usize::from(area.width);
    let body_text = view.body.text();
    let trailers = trailers(&body_text);
    let trailer_rows = u16::try_from(trailers.len().clamp(1, 3)).unwrap_or(3);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(3),
            Constraint::Length(1),
            Constraint::Min(3),
            Constraint::Length(1 + trailer_rows),
            Constraint::Length(1),
        ])
        .split(area);

    frame.render_widget(
        Paragraph::new(ruled_heading(theme, "Commit message", width, Role::Muted)),
        rows[0],
    );
    draw_type_chips(frame, view, theme, rows[1]);

    let length = view.subject.text().chars().count();
    let counter_role = match length {
        0..=50 => Role::Muted,
        51..=SUBJECT_LIMIT => Role::Mod,
        _ => Role::Del,
    };
    let counter = format!("{length}/{SUBJECT_LIMIT}");
    let label_width = width.saturating_sub(counter.len());
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                fit("Subject", label_width),
                label_style(theme, view.focus == Field::Subject),
            ),
            Span::styled(counter, theme.fg(counter_role)),
        ])),
        rows[3],
    );
    draw_text_box(
        frame,
        theme,
        &view.subject,
        rows[4],
        view.focus == Field::Subject,
    );
    frame.render_widget(
        Paragraph::new(Line::styled(
            "Body",
            label_style(theme, view.focus == Field::Body),
        )),
        rows[5],
    );
    draw_text_box(frame, theme, &view.body, rows[6], view.focus == Field::Body);

    let mut trailer_lines = vec![heading(theme, "Trailers")];
    if trailers.is_empty() {
        trailer_lines.push(Line::styled(
            "none · end the body with lines like Refs: #42",
            theme.fg(Role::Subtle),
        ));
    }
    for (token, value) in trailers.iter().take(3) {
        trailer_lines.push(Line::from(vec![
            Span::styled(format!("{token}: "), theme.fg(Role::Muted)),
            Span::styled(value.clone(), theme.fg(Role::Ink)),
        ]));
    }
    frame.render_widget(Paragraph::new(trailer_lines), rows[7]);
    draw_options(frame, view, theme, rows[8]);
}

fn label_style(theme: &Theme, focused: bool) -> ratatui::style::Style {
    if focused {
        theme.bold(Role::Cursor)
    } else {
        theme.fg(Role::Muted)
    }
}

/// `▸ [feat] fix refactor …` — the chip matching the subject's prefix is highlighted.
fn draw_type_chips(frame: &mut Frame<'_>, view: &CommitView, theme: &Theme, area: Rect) {
    let focused = view.focus == Field::Type;
    let current = subject_type(&view.subject.text());
    let mut spans = vec![Span::styled(
        if focused { "▸ " } else { "  " },
        theme.fg(Role::Cursor),
    )];
    for kind in TYPES {
        spans.push(if Some(kind) == current {
            Span::styled(format!("[{kind}]"), theme.bold(Role::Forge))
        } else {
            Span::styled(format!(" {kind} "), theme.fg(Role::Muted))
        });
    }
    if current.is_none() {
        spans.push(Span::styled("  no type", theme.fg(Role::Subtle)));
    }
    if focused {
        spans.push(Span::styled("  ←→ change", theme.fg(Role::Subtle)));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// A bordered input showing `text`, scrolled so the cursor stays visible.
fn draw_text_box(frame: &mut Frame<'_>, theme: &Theme, text: &TextArea, area: Rect, focused: bool) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .border_style(theme.fg(if focused {
            Role::Cursor
        } else {
            Role::LineStrong
        }))
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let (row, column) = text.cursor();
    let height = usize::from(inner.height);
    let width = usize::from(inner.width);
    let top = (row + 1).saturating_sub(height);
    let left = (column + 1).saturating_sub(width);
    let lines: Vec<Line> = text
        .lines()
        .iter()
        .skip(top)
        .take(height)
        .map(|line| {
            Line::styled(
                line.chars().skip(left).take(width).collect::<String>(),
                theme.fg(Role::Ink),
            )
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
    if focused {
        let x = inner.x + u16::try_from(column - left).unwrap_or(0);
        let y = inner.y + u16::try_from(row - top).unwrap_or(0);
        frame.set_cursor_position(Position::new(x, y));
    }
}

fn draw_options(frame: &mut Frame<'_>, view: &CommitView, theme: &Theme, area: Rect) {
    let focused = view.focus == Field::Options;
    let mut spans = vec![Span::styled(
        if focused { "▸ " } else { "  " },
        theme.fg(Role::Cursor),
    )];
    for (index, toggle) in Toggle::ORDER.iter().enumerate() {
        let mark = if view.toggled(*toggle) { "☑" } else { "☐" };
        let style = if focused && index == view.option {
            theme.bold(Role::Cursor).patch(theme.bg(Role::Selection))
        } else if view.toggled(*toggle) {
            theme.fg(Role::Ink)
        } else {
            theme.fg(Role::Muted)
        };
        spans.push(Span::styled(format!("{mark} {}", toggle.label()), style));
        spans.push(Span::raw("  "));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}
