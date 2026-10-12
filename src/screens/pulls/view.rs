use super::PullsView;
use crate::{
    github::pulls::{
        CheckState, Decision, Mergeable, PullDetail, PullRequest, PullState, ReviewState,
    },
    screens::Screen,
    theme::{Role, Theme},
    ui::{
        Hint, draw_bar, draw_tabs, draw_title_bar, field, fit, hint, key_line, pane_block, primary,
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
const AUTHOR_MIN_WIDTH: usize = 76;
const AUTHOR_WIDTH: usize = 12;
const CHECKS_WIDTH: usize = 7;
const REVIEW_WIDTH: usize = 8;
const AGE_WIDTH: usize = 4;
/// Description lines shown in the detail before it is cut with `…`.
const BODY_LINES: usize = 6;
/// Non-passing checks listed by name in the detail.
const CHECK_LINES: usize = 6;
const MIN_TITLE_WIDTH: usize = 12;

pub fn draw_pulls(frame: &mut Frame<'_>, view: &PullsView, theme: &Theme, user: Option<&str>) {
    let outer = window(theme);
    let inner = outer.inner(frame.area());
    frame.render_widget(outer, frame.area());

    let busy = view
        .busy
        .or((view.loading && view.pulls.is_some()).then_some("asking GitHub…"));
    let notice = status_line(theme, busy, view.notice.as_ref());
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
    let context = repo_context(theme, &view.path, view.tracking.as_ref());
    draw_title_bar(frame, theme, rows[0], context, user);
    draw_tabs(frame, theme, rows[1], Screen::PullRequests);
    draw_body(frame, view, theme, rows[2]);
    if let Some(notice) = notice {
        frame.render_widget(Paragraph::new(notice), rows[3]);
    }
    let width = usize::from(rows[4].width);
    draw_bar(frame, theme, rows[4], key_line(theme, &hints(view), width));
}

fn hints(view: &PullsView) -> Vec<Hint> {
    vec![
        primary("enter", "Open on GitHub"),
        hint("c", "Check out"),
        hint("f", view.filter.label()),
        hint("tab", "Details"),
        hint("esc", if view.detail_open { "List" } else { "Changes" }),
        hint("/", "Commands"),
        hint("r", "Refresh"),
        hint("q", "Quit"),
    ]
}

fn draw_body(frame: &mut Frame<'_>, view: &PullsView, theme: &Theme, area: Rect) {
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

fn draw_list(frame: &mut Frame<'_>, view: &PullsView, theme: &Theme, area: Rect) {
    let width = usize::from(area.width);
    let now = unix_now();
    // Remote data always says where it came from and how fresh it is.
    let freshness = match view.fetched {
        Some(at) if relative_age(now, at) == "now" => "from GitHub just now".to_owned(),
        Some(at) => format!("from GitHub {} ago", relative_age(now, at)),
        None => "from GitHub".to_owned(),
    };
    let count = view.list().len();
    let title = format!(
        "Pull requests · {} · {count}{} · {freshness}",
        view.filter.label(),
        if count >= crate::github::pulls::LIMIT {
            "+"
        } else {
            ""
        }
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
            format!("Could not load pull requests. {error} Press r to retry."),
            Role::Del,
        ))
    } else if view.pulls.is_none() {
        Some(("Asking GitHub…".to_owned(), Role::Muted))
    } else if view.list().is_empty() {
        Some((
            format!(
                "No {} pull requests. Press f to look at another state.",
                view.filter.label()
            ),
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

    let number_width = view
        .list()
        .iter()
        .map(|pull| pull.number.to_string().len() + 1)
        .max()
        .unwrap_or(2);
    let items: Vec<ListItem> = view
        .list()
        .iter()
        .enumerate()
        .map(|(index, pull)| {
            ListItem::new(pull_line(
                theme,
                pull,
                number_width,
                width,
                index == view.selected,
                now,
            ))
        })
        .collect();
    let mut state = ListState::default().with_selected(Some(view.selected));
    frame.render_stateful_widget(List::new(items), parts[1], &mut state);
}

/// `> #123 ● title                    ✓ 5/5  approved  author  3h`
fn pull_line(
    theme: &Theme,
    pull: &PullRequest,
    number_width: usize,
    width: usize,
    selected: bool,
    now: u64,
) -> Line<'static> {
    let show_author = width >= AUTHOR_MIN_WIDTH;
    let right = 1
        + CHECKS_WIDTH
        + 1
        + REVIEW_WIDTH
        + 1
        + AGE_WIDTH
        + if show_author { AUTHOR_WIDTH + 1 } else { 0 };
    let fixed = 2 + number_width + 1 + 2;
    let title_width = width.saturating_sub(fixed + right).max(MIN_TITLE_WIDTH);

    let (state, state_role) = match (pull.state, pull.draft) {
        (PullState::Merged, _) => ("◆", Role::Forge),
        (PullState::Closed, _) => ("✕", Role::Del),
        (PullState::Open, true) => ("○", Role::Muted),
        (PullState::Open, false) => ("●", Role::Add),
    };
    let (checks, checks_role) = checks_text(pull);
    let (review, review_role) = match pull.decision {
        Decision::Approved => ("approved", Role::Add),
        Decision::ChangesRequested => ("changes", Role::Del),
        Decision::ReviewRequired => ("review", Role::Mod),
        Decision::None => ("", Role::Muted),
    };
    let title_role = if pull.draft || pull.state != PullState::Open {
        Role::Muted
    } else {
        Role::Ink
    };

    let mut spans = vec![
        if selected {
            Span::styled("> ", theme.fg(Role::Cursor))
        } else {
            Span::raw("  ")
        },
        Span::styled(
            format!("{:<number_width$}", format!("#{}", pull.number)),
            theme.fg(Role::Muted),
        ),
        Span::raw(" "),
        Span::styled(state, theme.fg(state_role)),
        Span::raw(" "),
        Span::styled(fit(&pull.title, title_width), theme.fg(title_role)),
        Span::raw(" "),
        Span::styled(fit(&checks, CHECKS_WIDTH), theme.fg(checks_role)),
        Span::raw(" "),
        Span::styled(fit(review, REVIEW_WIDTH), theme.fg(review_role)),
    ];
    if show_author {
        spans.push(Span::styled(
            format!(" {}", fit(&pull.author, AUTHOR_WIDTH)),
            theme.fg(Role::Muted),
        ));
    }
    spans.push(Span::styled(
        format!(" {:>AGE_WIDTH$}", relative_age(now, pull.updated)),
        theme.fg(Role::Muted),
    ));
    let line = Line::from(spans);
    if selected {
        line.style(theme.bg(Role::Selection))
    } else {
        line
    }
}

/// `✕ 1/6` when something failed, `◐ 3/6` while checks run, `✓ 6/6` when all passed.
fn checks_text(pull: &PullRequest) -> (String, Role) {
    let total = pull.counted_checks();
    if total == 0 {
        return (String::new(), Role::Muted);
    }
    let failed = pull.count(CheckState::Failed);
    let pending = pull.count(CheckState::Pending);
    if failed > 0 {
        (format!("✕ {failed}/{total}"), Role::Del)
    } else if pending > 0 {
        (format!("◐ {}/{total}", total - pending), Role::Mod)
    } else {
        (format!("✓ {total}/{total}"), Role::Add)
    }
}

fn draw_detail(frame: &mut Frame<'_>, view: &PullsView, theme: &Theme, area: Rect) {
    let width = usize::from(area.width);
    let Some(pull) = view.selected_pull() else {
        frame.render_widget(
            Paragraph::new(ruled_heading(theme, "Pull request", width, Role::Muted)),
            area,
        );
        return;
    };
    let mut lines = vec![
        ruled_heading(
            theme,
            &format!("Pull request #{}", pull.number),
            width,
            Role::Muted,
        ),
        Line::raw(""),
        Line::styled(pull.title.clone(), theme.bold(Role::Ink)),
        Line::from(vec![
            Span::styled(state_word(pull), state_style(theme, pull)),
            Span::styled(
                format!(" · {} wants {} → {}", pull.author, pull.head, pull.base),
                theme.fg(Role::Muted),
            ),
        ]),
    ];
    if !pull.labels.is_empty() {
        let labels: Vec<String> = pull.labels.iter().map(|l| format!("[{l}]")).collect();
        lines.push(Line::styled(labels.join(" "), theme.fg(Role::Subtle)));
    }
    lines.push(Line::raw(""));
    lines.push(field(theme, "checks", checks_summary(pull)));
    lines.push(field(theme, "review", decision_text(pull.decision)));
    match view.details.get(&pull.number) {
        None => lines.push(Line::styled("Reading details…", theme.fg(Role::Muted))),
        Some(Err(error)) => lines.push(Line::styled(
            format!("Could not read the details. {error} Press r to retry."),
            theme.fg(Role::Del),
        )),
        Some(Ok(detail)) => lines.extend(detail_lines(theme, pull, detail, width)),
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
}

fn state_word(pull: &PullRequest) -> &'static str {
    match (pull.state, pull.draft) {
        (PullState::Merged, _) => "merged",
        (PullState::Closed, _) => "closed",
        (PullState::Open, true) => "draft",
        (PullState::Open, false) => "open",
    }
}

fn state_style(theme: &Theme, pull: &PullRequest) -> ratatui::style::Style {
    theme.bold(match (pull.state, pull.draft) {
        (PullState::Merged, _) => Role::Forge,
        (PullState::Closed, _) => Role::Del,
        (PullState::Open, true) => Role::Muted,
        (PullState::Open, false) => Role::Add,
    })
}

fn checks_summary(pull: &PullRequest) -> String {
    let total = pull.counted_checks();
    if total == 0 {
        return "none reported".to_owned();
    }
    let mut parts = vec![format!("{} passed", pull.count(CheckState::Passed))];
    for (state, word) in [
        (CheckState::Failed, "failed"),
        (CheckState::Pending, "running"),
    ] {
        let count = pull.count(state);
        if count > 0 {
            parts.push(format!("{count} {word}"));
        }
    }
    parts.join(" · ")
}

fn decision_text(decision: Decision) -> String {
    match decision {
        Decision::Approved => "approved",
        Decision::ChangesRequested => "changes requested",
        Decision::ReviewRequired => "review required",
        Decision::None => "no review required",
    }
    .to_owned()
}

fn detail_lines(
    theme: &Theme,
    pull: &PullRequest,
    detail: &PullDetail,
    width: usize,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    lines.push(field(
        theme,
        "merge",
        match detail.mergeable {
            Mergeable::Clean => "no conflicts with the base branch",
            Mergeable::Conflicting => "has conflicts with the base branch",
            Mergeable::Unknown => "GitHub is still working it out",
        }
        .to_owned(),
    ));
    lines.push(field(
        theme,
        "size",
        format!(
            "+{} −{} in {} file{} · {} commit{} · {} comment{}",
            pull.additions,
            pull.deletions,
            pull.changed_files,
            plural(pull.changed_files),
            detail.commits,
            plural(detail.commits),
            detail.comments,
            plural(detail.comments),
        ),
    ));

    let body = detail.body.trim();
    if !body.is_empty() {
        lines.push(Line::raw(""));
        let all: Vec<&str> = body.lines().collect();
        lines.extend(
            all.iter()
                .take(BODY_LINES)
                .map(|line| Line::raw((*line).to_owned())),
        );
        if all.len() > BODY_LINES {
            lines.push(Line::styled(
                format!("… {} more lines on GitHub", all.len() - BODY_LINES),
                theme.fg(Role::Muted),
            ));
        }
    }

    let unhappy: Vec<_> = pull
        .checks
        .iter()
        .filter(|check| matches!(check.state, CheckState::Failed | CheckState::Pending))
        .collect();
    if !unhappy.is_empty() {
        lines.push(Line::raw(""));
        lines.push(ruled_heading(
            theme,
            "Checks needing attention",
            width,
            Role::Muted,
        ));
        for check in unhappy.iter().take(CHECK_LINES) {
            let (glyph, role) = if check.state == CheckState::Failed {
                ("✕", Role::Del)
            } else {
                ("◐", Role::Mod)
            };
            let name = match &check.workflow {
                Some(workflow) => format!("{workflow} / {}", check.name),
                None => check.name.clone(),
            };
            lines.push(Line::from(vec![
                Span::styled(format!("{glyph} "), theme.fg(role)),
                Span::raw(name),
            ]));
        }
        if unhappy.len() > CHECK_LINES {
            lines.push(Line::styled(
                format!("… and {} more", unhappy.len() - CHECK_LINES),
                theme.fg(Role::Muted),
            ));
        }
    }

    if !detail.reviews.is_empty() || !detail.requested.is_empty() {
        lines.push(Line::raw(""));
        lines.push(ruled_heading(theme, "Reviews", width, Role::Muted));
        let now = unix_now();
        for review in &detail.reviews {
            let (glyph, word, role) = match review.state {
                ReviewState::Approved => ("✓", "approved", Role::Add),
                ReviewState::ChangesRequested => ("✕", "requested changes", Role::Del),
                ReviewState::Commented => ("•", "commented", Role::Muted),
                ReviewState::Dismissed => ("–", "review dismissed", Role::Muted),
                ReviewState::Pending => ("◐", "has a pending review", Role::Mod),
            };
            lines.push(Line::from(vec![
                Span::styled(format!("{glyph} "), theme.fg(role)),
                Span::raw(format!("{} {word}", review.author)),
                Span::styled(
                    format!(" · {} ago", relative_age(now, review.submitted)),
                    theme.fg(Role::Muted),
                ),
            ]));
        }
        for name in &detail.requested {
            lines.push(Line::from(vec![
                Span::styled("◌ ", theme.fg(Role::Muted)),
                Span::raw(format!("{name} was asked to review")),
            ]));
        }
    }

    if !detail.files.is_empty() {
        lines.push(Line::raw(""));
        lines.push(ruled_heading(
            theme,
            &format!("Files · {}", detail.files.len()),
            width,
            Role::Muted,
        ));
        for file in &detail.files {
            lines.push(Line::from(vec![
                Span::styled(format!("+{}", file.additions), theme.fg(Role::Add)),
                Span::raw(" "),
                Span::styled(format!("−{}", file.deletions), theme.fg(Role::Del)),
                Span::raw("  "),
                Span::raw(file.path.clone()),
            ]));
        }
    }
    lines
}

fn plural(count: usize) -> &'static str {
    if count == 1 { "" } else { "s" }
}
