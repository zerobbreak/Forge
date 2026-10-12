use super::{BranchesView, DeleteChoice, DirtyChoice, Mode, NameBox, NamePurpose};
use crate::{
    git::branches::{Branch, BranchKind, BranchList},
    screens::Screen,
    theme::{Role, Theme},
    ui::{
        Hint, draw_bar, draw_tabs, draw_title_bar, field, fit, format_utc, hint, key_line,
        pane_block, popup, primary, relative_age, repo_context, ruled_heading, status_line,
        unix_now, window,
    },
};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Position, Rect},
    text::{Line, Span},
    widgets::{
        Block, BorderType, Borders, Clear, List, ListItem, ListState, Padding, Paragraph, Wrap,
    },
};

/// At this width the list and the detail sit side by side; below it one replaces the other.
const SPLIT_MIN_WIDTH: u16 = 100;
const TRACK_WIDTH: usize = 8;
const AGE_WIDTH: usize = 4;
/// Columns taken by everything in a row except the name and the subject.
const ROW_CHROME: usize = 2 + 2 + 1 + TRACK_WIDTH + 1 + AGE_WIDTH;
const MIN_NAME_WIDTH: usize = 8;
const MIN_SUBJECT_WIDTH: usize = 12;

pub fn draw_branches(
    frame: &mut Frame<'_>,
    view: &BranchesView,
    theme: &Theme,
    user: Option<&str>,
) {
    let outer = window(theme);
    let inner = outer.inner(frame.area());
    frame.render_widget(outer, frame.area());

    let busy = view.busy.or(view.loading.then_some("reading branches…"));
    let notice = status_line(
        theme,
        busy.filter(|_| view.busy.is_some() || view.list.is_some()),
        view.notice.as_ref(),
    );
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
    draw_tabs(frame, theme, rows[1], Screen::Branches);
    draw_body(frame, view, theme, rows[2]);
    if let Some(notice) = notice {
        frame.render_widget(Paragraph::new(notice), rows[3]);
    }
    let width = usize::from(rows[4].width);
    draw_bar(frame, theme, rows[4], key_line(theme, &hints(view), width));

    match &view.mode {
        Mode::Browse => {}
        Mode::Name(input) => draw_name(frame, theme, input, inner),
        Mode::Delete(choice) => draw_delete(frame, theme, choice, inner),
        Mode::Dirty(choice) => draw_dirty(frame, theme, choice, inner),
    }
}

fn hints(view: &BranchesView) -> Vec<Hint> {
    let mut hints = vec![primary("enter", "Switch"), hint("n", "New")];
    if view
        .selected_branch()
        .is_some_and(|branch| branch.kind == BranchKind::Local)
    {
        hints.extend([hint("R", "Rename"), hint("x", "Delete")]);
    }
    hints.extend([
        hint("tab", "Details"),
        hint("esc", if view.detail_open { "List" } else { "Changes" }),
        hint("r", "Refresh"),
        hint("q", "Quit"),
    ]);
    hints
}

fn draw_body(frame: &mut Frame<'_>, view: &BranchesView, theme: &Theme, area: Rect) {
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

fn draw_list(frame: &mut Frame<'_>, view: &BranchesView, theme: &Theme, area: Rect) {
    let width = usize::from(area.width);
    let message = if let Some(error) = &view.load_error {
        Some((
            format!("Could not read the branches. {error} Press r to retry."),
            Role::Del,
        ))
    } else if view.list.is_none() {
        Some(("Reading branches…".to_owned(), Role::Muted))
    } else if view.branches().is_empty() {
        Some((
            "No branches yet. Make the first commit in Changes (press c) and your branch will appear here."
                .to_owned(),
            Role::Muted,
        ))
    } else {
        None
    };
    if let Some((text, role)) = message {
        frame.render_widget(
            Paragraph::new(vec![
                ruled_heading(theme, "Branches", width, Role::Cursor),
                Line::raw(""),
                Line::styled(text, theme.fg(role)),
            ])
            .wrap(Wrap { trim: true }),
            area,
        );
        return;
    }
    let Some(list) = view.list.as_ref() else {
        return;
    };

    let name_width = list
        .branches
        .iter()
        .map(|branch| branch.name.chars().count())
        .max()
        .unwrap_or(0)
        .clamp(MIN_NAME_WIDTH, (width * 45 / 100).max(MIN_NAME_WIDTH))
        .min(width.saturating_sub(ROW_CHROME).max(MIN_NAME_WIDTH));
    let now = unix_now();

    let mut items = Vec::new();
    let mut selected_row = 0;
    let locals = list.local_count();
    for (index, branch) in list.branches.iter().enumerate() {
        if index == 0 || index == locals {
            let (title, count) = if index < locals {
                ("Local", locals)
            } else {
                ("Remote", list.branches.len() - locals)
            };
            let mut title = format!("{title} · {count}");
            if index == 0
                && let Some(short) = &list.detached
            {
                title.push_str(&format!(" · HEAD detached at {short}"));
            }
            if index > 0 {
                items.push(ListItem::new(Line::raw("")));
            }
            let role = if index == 0 {
                Role::Cursor
            } else {
                Role::Muted
            };
            items.push(ListItem::new(ruled_heading(theme, &title, width, role)));
        }
        if index == view.selected {
            selected_row = items.len();
        }
        items.push(ListItem::new(branch_line(
            theme,
            branch,
            name_width,
            width,
            index == view.selected,
            now,
        )));
    }
    let mut state = ListState::default().with_selected(Some(selected_row));
    frame.render_stateful_widget(List::new(items), area, &mut state);
}

/// `> ● name  in sync  3h  subject`
fn branch_line(
    theme: &Theme,
    branch: &Branch,
    name_width: usize,
    width: usize,
    selected: bool,
    now: u64,
) -> Line<'static> {
    let (track, role) = track_text(branch);
    let mut spans = vec![
        if selected {
            Span::styled("> ", theme.fg(Role::Cursor))
        } else {
            Span::raw("  ")
        },
        if branch.current {
            Span::styled("● ", theme.fg(Role::Add))
        } else {
            Span::raw("  ")
        },
        Span::styled(
            fit(&branch.name, name_width),
            if branch.current {
                theme.bold(Role::Ink)
            } else if branch.kind == BranchKind::Remote {
                theme.fg(Role::Muted)
            } else {
                theme.fg(Role::Ink)
            },
        ),
        Span::raw(" "),
        Span::styled(fit(&track, TRACK_WIDTH), theme.fg(role)),
        Span::styled(
            format!(" {:>AGE_WIDTH$}", relative_age(now, branch.time)),
            theme.fg(Role::Muted),
        ),
    ];
    let used = ROW_CHROME - 2 - 2 + name_width + 4;
    let room = width.saturating_sub(used + 2 + 2);
    if room > MIN_SUBJECT_WIDTH {
        spans.push(Span::styled(
            format!("  {}", fit(&branch.subject, room - 2)),
            theme.fg(Role::Subtle),
        ));
    }
    let line = Line::from(spans);
    if selected {
        line.style(theme.bg(Role::Selection))
    } else {
        line
    }
}

/// The short upstream status shown in a row, and its colour.
fn track_text(branch: &Branch) -> (String, Role) {
    if branch.kind == BranchKind::Remote {
        return (String::new(), Role::Muted);
    }
    if branch.worktree.is_some() {
        return ("worktree".to_owned(), Role::Mod);
    }
    if branch.gone {
        return ("gone".to_owned(), Role::Del);
    }
    if branch.upstream.is_none() {
        return ("local".to_owned(), Role::Muted);
    }
    match (branch.ahead, branch.behind) {
        (0, 0) => ("in sync".to_owned(), Role::Muted),
        (ahead, 0) => (format!("↑{ahead}"), Role::Add),
        (0, behind) => (format!("↓{behind}"), Role::Del),
        (ahead, behind) => (format!("↑{ahead} ↓{behind}"), Role::Mod),
    }
}

fn draw_detail(frame: &mut Frame<'_>, view: &BranchesView, theme: &Theme, area: Rect) {
    let width = usize::from(area.width);
    let (Some(list), Some(branch)) = (view.list.as_ref(), view.selected_branch()) else {
        frame.render_widget(
            Paragraph::new(ruled_heading(theme, "Branch", width, Role::Muted)),
            area,
        );
        return;
    };
    let into = view.current_name();
    let mut lines = vec![
        ruled_heading(
            theme,
            &format!("Branch · {}", branch.name),
            width,
            Role::Muted,
        ),
        Line::raw(""),
        field(theme, "tip", format!("{} {}", branch.short, branch.subject)),
        field(
            theme,
            "updated",
            format!(
                "{} · {} ago",
                format_utc(branch.time),
                relative_age(unix_now(), branch.time)
            ),
        ),
    ];
    lines.push(field(theme, "upstream", upstream_text(branch)));
    lines.push(field(theme, "vs HEAD", compare_text(branch, &into)));
    if let Some(folder) = &branch.worktree {
        lines.push(field(theme, "worktree", format!("checked out in {folder}")));
    }
    lines.push(Line::raw(""));
    lines.extend(worktree_lines(theme, list, width));
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
}

fn upstream_text(branch: &Branch) -> String {
    if branch.kind == BranchKind::Remote {
        return "this is a remote branch".to_owned();
    }
    match (&branch.upstream, branch.gone) {
        (None, _) => "none (local only)".to_owned(),
        (Some(upstream), true) => format!("{upstream} was deleted on the remote"),
        (Some(upstream), false) => {
            format!("{upstream} · ↑{} ↓{}", branch.ahead, branch.behind)
        }
    }
}

fn compare_text(branch: &Branch, into: &str) -> String {
    if branch.current {
        return "this is the checked-out branch".to_owned();
    }
    match branch.unique {
        Some(0) => format!("every commit is already in {into}"),
        Some(1) => format!("1 commit is not in {into}"),
        Some(count) => format!("{count} commits are not in {into}"),
        None if branch.merged => format!("merged into {into}"),
        None => format!("not merged into {into}"),
    }
}

fn worktree_lines(theme: &Theme, list: &BranchList, width: usize) -> Vec<Line<'static>> {
    let mut lines = vec![ruled_heading(
        theme,
        &format!("Worktrees · {}", list.worktrees.len()),
        width,
        Role::Muted,
    )];
    if list.worktrees.is_empty() {
        lines.push(Line::styled("None found.", theme.fg(Role::Muted)));
    }
    for tree in &list.worktrees {
        let what = match (&tree.branch, tree.bare, tree.detached) {
            (_, true, _) => "(bare)".to_owned(),
            (Some(branch), _, _) => format!("⎇ {branch}"),
            (None, _, _) => format!("(detached {})", tree.short),
        };
        lines.push(Line::from(vec![
            if tree.current {
                Span::styled("● ", theme.fg(Role::Add))
            } else {
                Span::raw("  ")
            },
            Span::styled(what, theme.fg(Role::Ink)),
        ]));
        lines.push(Line::styled(
            format!("  {}", tree.path),
            theme.fg(Role::Muted),
        ));
    }
    lines
}

/// A bordered dialog with `lines` and a hint row, centred near the top of `area`.
fn draw_dialog(
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

fn draw_name(frame: &mut Frame<'_>, theme: &Theme, input: &NameBox, area: Rect) {
    let (title, intro, action) = match &input.purpose {
        NamePurpose::Create { from, .. } => (
            "New branch",
            format!("Starting from {from}. Name the new branch:"),
            "Create",
        ),
        NamePurpose::Rename { old } => ("Rename branch", format!("New name for {old}:"), "Rename"),
    };
    let mut lines = vec![
        Line::styled(intro, theme.fg(Role::Muted)),
        Line::raw(""),
        Line::raw(""),
    ];
    match &input.error {
        Some(error) => lines.push(Line::styled(format!("✕ {error}"), theme.fg(Role::Del))),
        None => lines.push(Line::styled(
            "Letters, digits, / - _ . are fine. No spaces.",
            theme.fg(Role::Muted),
        )),
    }
    let body = draw_dialog(
        frame,
        theme,
        area,
        title,
        Role::Cursor,
        lines,
        &[primary("enter", action), hint("esc", "Cancel")],
    );

    // The text sits on the third row, scrolled so the cursor stays visible.
    let field_area = Rect {
        y: body.y + 2,
        height: 1,
        ..body
    };
    let text = input.text.text();
    let (_, column) = input.text.cursor();
    let room = usize::from(field_area.width).saturating_sub(3).max(1);
    let start = column.saturating_sub(room);
    let visible: String = text.chars().skip(start).take(room).collect();
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("▸ ", theme.fg(Role::Cursor)),
            Span::styled(visible, theme.bold(Role::Ink)),
        ]))
        .style(theme.bg(Role::Raised)),
        field_area,
    );
    frame.set_cursor_position(Position::new(
        field_area.x + 2 + (column - start) as u16,
        field_area.y,
    ));
}

fn draw_delete(frame: &mut Frame<'_>, theme: &Theme, choice: &DeleteChoice, area: Rect) {
    let mut lines = vec![Line::styled(
        format!("Delete branch {}?", choice.name),
        theme.bold(Role::Ink),
    )];
    if choice.merged {
        lines.push(Line::styled(
            format!(
                "Every commit on it is already in {}, so nothing is lost.",
                choice.into
            ),
            theme.fg(Role::Add),
        ));
    } else {
        let unique = match choice.unique {
            Some(1) => "1 commit that is".to_owned(),
            Some(count) => format!("{count} commits that are"),
            None => "commits that may be".to_owned(),
        };
        lines.push(Line::from(vec![
            Span::styled("You will lose: ", theme.fg(Role::Muted)),
            Span::styled(
                format!("{unique} in no other branch ({})", choice.into),
                theme.fg(Role::Del),
            ),
        ]));
    }
    lines.push(Line::raw(""));
    lines.push(Line::styled(
        format!(
            "You can bring it back with: git branch {} {}",
            choice.name, choice.short
        ),
        theme.fg(Role::Muted),
    ));
    draw_dialog(
        frame,
        theme,
        area,
        "Delete branch",
        Role::Del,
        lines,
        &[primary("y", "Delete"), hint("esc", "Keep")],
    );
}

fn draw_dirty(frame: &mut Frame<'_>, theme: &Theme, choice: &DirtyChoice, area: Rect) {
    let files = if choice.files == 1 {
        "1 file has".to_owned()
    } else {
        format!("{} files have", choice.files)
    };
    let lines = vec![
        Line::styled(
            format!("{files} uncommitted changes."),
            theme.bold(Role::Ink),
        ),
        Line::styled(
            format!("Switching to {}:", choice.target.name),
            theme.fg(Role::Muted),
        ),
        Line::raw(""),
        Line::from(vec![
            Span::styled("s ", theme.bold(Role::Forge)),
            Span::raw("stash them, switch, and restore later with git stash pop"),
        ]),
        Line::from(vec![
            Span::styled("c ", theme.bold(Role::Forge)),
            Span::raw("carry them over (Git refuses if they would be overwritten)"),
        ]),
    ];
    draw_dialog(
        frame,
        theme,
        area,
        "Uncommitted changes",
        Role::Mod,
        lines,
        &[
            primary("s", "Stash & switch"),
            hint("c", "Carry over"),
            hint("esc", "Cancel"),
        ],
    );
}
