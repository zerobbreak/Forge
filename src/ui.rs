//! Widgets and text helpers shared by every TUI screen.

use crate::{
    git::repository::BranchTracking,
    screens::Screen,
    theme::{Role, Theme},
};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Padding, Paragraph, Wrap},
};
use std::{
    env,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

/// A key and what it does, shown in a screen's key bar.
#[derive(Clone, Copy)]
pub struct Hint {
    pub key: &'static str,
    pub label: &'static str,
    pub primary: bool,
}

pub const fn hint(key: &'static str, label: &'static str) -> Hint {
    Hint {
        key,
        label,
        primary: false,
    }
}

pub const fn primary(key: &'static str, label: &'static str) -> Hint {
    Hint {
        key,
        label,
        primary: true,
    }
}

/// `key label   key label …`, dropping trailing hints that would not fit in `width`.
pub fn key_line(theme: &Theme, hints: &[Hint], width: usize) -> Line<'static> {
    let mut spans = vec![Span::raw(" ")];
    let mut used = 1;
    for (index, hint) in hints.iter().enumerate() {
        let separator = if index == 0 { "" } else { "   " };
        let key = if hint.primary {
            format!(" {} ", hint.key)
        } else {
            hint.key.to_owned()
        };
        let cost = separator.chars().count() + key.chars().count() + 1 + hint.label.chars().count();
        if index > 0 && used + cost > width {
            break;
        }
        used += cost;
        spans.push(Span::raw(separator));
        spans.push(Span::styled(
            key,
            if hint.primary {
                theme.primary()
            } else {
                theme.bold(Role::Forge)
            },
        ));
        spans.push(Span::raw(format!(" {}", hint.label)));
    }
    Line::from(spans)
}

/// A one-row bar on the raised surface: title bars, key bars and notices.
pub fn draw_bar(frame: &mut Frame<'_>, theme: &Theme, area: Rect, line: Line<'static>) {
    frame.render_widget(
        Paragraph::new(line).style(theme.bg(Role::Raised).patch(theme.fg(Role::Ink))),
        area,
    );
}

/// A one-line message under a screen's panes. Errors read: what happened, why,
/// and what to do next.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    pub text: String,
    pub error: bool,
}

impl Notice {
    pub fn info(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            error: false,
        }
    }

    pub fn error(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            error: true,
        }
    }
}

/// The status line: `◐ busy…` while work runs, else `✓`/`✕` and the notice.
pub fn status_line(
    theme: &Theme,
    busy: Option<&str>,
    notice: Option<&Notice>,
) -> Option<Line<'static>> {
    if let Some(label) = busy {
        return Some(Line::styled(format!(" ◐ {label}"), theme.fg(Role::Mod)));
    }
    let notice = notice?;
    Some(if notice.error {
        Line::styled(format!(" ✕ {}", notice.text), theme.fg(Role::Del))
    } else {
        Line::styled(format!(" ✓ {}", notice.text), theme.fg(Role::Muted))
    })
}

/// Title-bar context for a repository: path, `⎇ branch`, and `↑ahead ↓behind`.
pub fn repo_context(
    theme: &Theme,
    path: &Path,
    tracking: Option<&BranchTracking>,
) -> Vec<Span<'static>> {
    let mut spans = vec![
        Span::styled(display_root(path).replace('\\', "/"), theme.fg(Role::Muted)),
        Span::raw("  "),
    ];
    let Some(tracking) = tracking else {
        spans.push(Span::styled("reading…", theme.fg(Role::Muted)));
        return spans;
    };
    let branch = if tracking.branch.is_empty() {
        "(detached)".to_owned()
    } else {
        tracking.branch.clone()
    };
    spans.push(Span::styled("⎇ ", theme.fg(Role::Muted)));
    spans.push(Span::styled(branch, theme.bold(Role::Ink)));
    if tracking.upstream {
        spans.push(Span::styled(
            format!(" ↑{}", tracking.ahead),
            theme.fg(Role::Add),
        ));
        spans.push(Span::styled(
            format!(" ↓{}", tracking.behind),
            theme.fg(if tracking.behind > 0 {
                Role::Del
            } else {
                Role::Muted
            }),
        ));
    }
    spans
}

/// The rounded outer frame every full screen sits in.
pub fn window(theme: &Theme) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme.fg(Role::LineStrong))
        .style(theme.bg(Role::Panel).patch(theme.fg(Role::Ink)))
}

/// A pane with a rule on its left edge when it follows another pane.
pub fn pane_block(theme: &Theme, divided: bool) -> Block<'static> {
    Block::default()
        .borders(if divided {
            Borders::LEFT
        } else {
            Borders::NONE
        })
        .border_style(theme.fg(Role::Line))
        .padding(Padding::new(1, 1, 1, 0))
}

/// Title bar: `FORGE` plus `context` on the left, the GitHub login on the right.
pub fn draw_title_bar(
    frame: &mut Frame<'_>,
    theme: &Theme,
    area: Rect,
    context: Vec<Span<'static>>,
    user: Option<&str>,
) {
    frame.render_widget(Block::default().style(theme.bg(Role::Raised)), area);
    let mut left = vec![
        Span::raw(" "),
        Span::styled("FORGE", theme.bold(Role::Forge)),
        Span::raw("  "),
    ];
    left.extend(context);
    let right = user.map(|user| {
        Line::from(vec![
            Span::styled("  ● ", theme.fg(Role::Add)),
            Span::styled(format!("@{user} "), theme.fg(Role::Ink)),
        ])
    });
    let right_width = right.as_ref().map_or(0, |line| line.width() as u16);
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(0), Constraint::Length(right_width)])
        .split(area);
    frame.render_widget(Paragraph::new(Line::from(left)), columns[0]);
    if let Some(right) = right {
        frame.render_widget(Paragraph::new(right), columns[1]);
    }
}

/// The one-row tab strip shared by Changes, Commits and Branches: `[1 Changes]  2 Commits …`.
pub fn draw_tabs(frame: &mut Frame<'_>, theme: &Theme, area: Rect, active: Screen) {
    let mut spans = vec![Span::raw(" ")];
    for (index, (screen, label)) in Screen::TABS.iter().enumerate() {
        let text = format!("{} {label}", index + 1);
        if *screen == active {
            // Brackets keep the active tab visible when colour is off.
            spans.push(Span::styled(format!("[{text}]"), theme.bold(Role::Cursor)));
        } else {
            spans.push(Span::styled(format!(" {text} "), theme.fg(Role::Muted)));
        }
        spans.push(Span::raw(" "));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// A bordered dialog with `lines` and a hint row, centred near the top of `area`.
pub fn draw_dialog(
    frame: &mut Frame<'_>,
    theme: &Theme,
    area: Rect,
    title: &str,
    border: Role,
    lines: Vec<Line<'static>>,
    hints: &[Hint],
) -> Rect {
    let height = u16::try_from(lines.len())
        .unwrap_or(u16::MAX)
        .saturating_add(3);
    let popup = popup(area, 68, height);
    frame.render_widget(Clear, popup);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme.fg(border))
        .title(Span::styled(
            format!(" {} ", title.to_uppercase()),
            theme.bold(border),
        ))
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
    let width = usize::from(sections[1].width);
    draw_bar(frame, theme, sections[1], key_line(theme, hints, width));
    sections[0]
}

/// An uppercase section label, e.g. `RECENT COMMITS`.
pub fn heading(theme: &Theme, text: &str) -> Line<'static> {
    Line::styled(text.to_uppercase(), theme.bold(Role::Muted))
}

/// An uppercase section label followed by a rule to `width`, e.g. `STAGED · 1 ─────`.
pub fn ruled_heading(theme: &Theme, text: &str, width: usize, role: Role) -> Line<'static> {
    let label = text.to_uppercase();
    let rule = width.saturating_sub(label.chars().count() + 1);
    Line::from(vec![
        Span::styled(label, theme.bold(role)),
        Span::raw(" "),
        Span::styled("─".repeat(rule), theme.fg(Role::Line)),
    ])
}

/// A `name   value` row of a detail pane.
pub fn field(theme: &Theme, name: &str, value: String) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{name:<10}"), theme.fg(Role::Muted)),
        Span::styled(value, theme.fg(Role::Ink)),
    ])
}

/// Pads or truncates `text` to exactly `width` characters, ending in `…` when cut.
pub fn fit(text: &str, width: usize) -> String {
    let length = text.chars().count();
    if length <= width {
        format!("{text:<width$}")
    } else if width == 0 {
        String::new()
    } else {
        let mut cut: String = text.chars().take(width - 1).collect();
        cut.push('…');
        cut
    }
}

/// A rectangle of at most `width` × `height` centred horizontally in `area`, near its top.
pub fn popup(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width.saturating_sub(4));
    let height = height.min(area.height);
    Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + 2.min(area.height.saturating_sub(height)),
        width,
        height,
    }
}

/// Compact age: `now`, `5m`, `3h`, `2d`, `1w`, `4mo`, `1y`.
pub fn relative_age(now: u64, then: u64) -> String {
    const MINUTE: u64 = 60;
    const HOUR: u64 = 60 * MINUTE;
    const DAY: u64 = 24 * HOUR;
    let elapsed = now.saturating_sub(then);
    match elapsed {
        0..MINUTE => "now".to_owned(),
        MINUTE..HOUR => format!("{}m", elapsed / MINUTE),
        HOUR..DAY => format!("{}h", elapsed / HOUR),
        _ if elapsed < 7 * DAY => format!("{}d", elapsed / DAY),
        _ if elapsed < 30 * DAY => format!("{}w", elapsed / (7 * DAY)),
        _ if elapsed < 365 * DAY => format!("{}mo", elapsed / (30 * DAY)),
        _ => format!("{}y", elapsed / (365 * DAY)),
    }
}

pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

/// `2026-10-10 14:03 UTC` for a Unix timestamp.
pub fn format_utc(timestamp: u64) -> String {
    let days = (timestamp / 86_400) as i64;
    let seconds = timestamp % 86_400;
    // Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02} UTC",
        seconds / 3_600,
        seconds % 3_600 / 60
    )
}

/// `path` with the home folder shown as `~`.
pub fn display_root(root: &Path) -> String {
    if let Some(home) = env::var_os("USERPROFILE").or_else(|| env::var_os("HOME")) {
        let home = PathBuf::from(home);
        if let Ok(relative) = root.strip_prefix(home) {
            return format!("~/{}", relative.display());
        }
    }
    root.display().to_string()
}

#[cfg(test)]
mod tests {
    use super::{fit, format_utc, hint, key_line, primary, relative_age};
    use crate::theme::Theme;

    #[test]
    fn formats_compact_commit_ages() {
        assert_eq!(relative_age(1_000, 990), "now");
        assert_eq!(relative_age(10_000, 10_000 - 120), "2m");
        assert_eq!(relative_age(100_000, 100_000 - 3 * 3600), "3h");
        assert_eq!(relative_age(10_000_000, 10_000_000 - 8 * 86_400), "1w");
        assert_eq!(relative_age(100_000_000, 100_000_000 - 400 * 86_400), "1y");
        assert_eq!(relative_age(5, 10), "now");
    }

    #[test]
    fn fits_text_to_width() {
        assert_eq!(fit("abc", 5), "abc  ");
        assert_eq!(fit("abcdef", 4), "abc…");
        assert_eq!(fit("abc", 0), "");
    }

    #[test]
    fn key_line_drops_hints_that_do_not_fit() {
        let theme = Theme::new(false);
        let hints = [primary("s", "Stage"), hint("a", "All"), hint("q", "Quit")];
        let full = key_line(&theme, &hints, 80).to_string();
        assert_eq!(full, "  s  Stage   a All   q Quit");
        let narrow = key_line(&theme, &hints, 20).to_string();
        assert_eq!(narrow, "  s  Stage   a All");
    }

    #[test]
    fn formats_timestamps_as_utc_dates() {
        assert_eq!(format_utc(0), "1970-01-01 00:00 UTC");
        assert_eq!(format_utc(1_760_105_000), "2025-10-10 14:03 UTC");
        assert_eq!(format_utc(951_782_400), "2000-02-29 00:00 UTC");
    }
}
