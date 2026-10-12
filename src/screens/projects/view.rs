use super::{AccountPicker, PROJECTS_DIR_VARIABLE, Project, ProjectBrowser, Scope};
use crate::{
    theme::{Role, Theme},
    ui::{
        display_root, draw_bar, draw_title_bar, fit, heading, hint, key_line, popup, primary,
        relative_age, unix_now, window,
    },
};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::Modifier,
    text::{Line, Span},
    widgets::{
        Block, BorderType, Borders, Clear, List, ListItem, ListState, Padding, Paragraph, Wrap,
    },
};

/// Column widths for the project table; the branch column is dropped when narrow.
struct Columns {
    name: usize,
    branch: Option<usize>,
}

impl Columns {
    // Marker, key, ahead/behind, dirty, ci, last and their single-space gaps.
    const FIXED: usize = 2 + 2 + 1 + 7 + 1 + 5 + 1 + 2 + 1 + 4;

    fn for_width(width: usize) -> Self {
        let remaining = width.saturating_sub(Self::FIXED);
        if remaining >= 20 {
            let branch = remaining * 2 / 5;
            Self {
                name: remaining - branch - 1,
                branch: Some(branch),
            }
        } else {
            Self {
                name: remaining.max(4),
                branch: None,
            }
        }
    }
}

pub fn draw_browser(frame: &mut Frame<'_>, app: &ProjectBrowser, theme: &Theme) {
    let outer = window(theme);
    let inner = outer.inner(frame.area());
    frame.render_widget(outer, frame.area());

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(3),
            Constraint::Length(1),
        ])
        .split(inner);
    draw_title(frame, app, theme, rows[0]);

    let visible = app.visible_projects();
    if rows[1].width >= 70 {
        let list_share = if rows[1].width < 100 { 65 } else { 63 };
        let panes = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(list_share),
                Constraint::Percentage(100 - list_share),
            ])
            .split(rows[1]);
        draw_project_list(frame, app, theme, panes[0], &visible, true);
        draw_project_detail(frame, app, theme, panes[1], &visible);
    } else {
        draw_project_list(frame, app, theme, rows[1], &visible, false);
    }

    draw_key_bar(frame, app, theme, rows[2]);

    if let Some(picker) = &app.account_picker {
        draw_account_picker(frame, theme, picker, inner);
    }
}

fn draw_account_picker(frame: &mut Frame<'_>, theme: &Theme, picker: &AccountPicker, area: Rect) {
    let rows = picker
        .accounts
        .as_ref()
        .map_or(1, |accounts| accounts.len().max(1));
    let height = u16::try_from(rows).unwrap_or(u16::MAX).saturating_add(6);
    let popup = popup(area, 56, height);
    frame.render_widget(Clear, popup);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme.fg(Role::Cursor))
        .title(Span::styled(" SWITCH ACCOUNT ", theme.bold(Role::Forge)))
        .style(theme.bg(Role::Panel).patch(theme.fg(Role::Ink)));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(inner);

    let body_width = usize::from(sections[0].width.saturating_sub(2));
    let mut lines = vec![heading(theme, " github.com")];
    match (&picker.accounts, &picker.error) {
        (_, Some(error)) => {
            lines.push(Line::styled(
                " Could not use GitHub CLI accounts.",
                theme.fg(Role::Del),
            ));
            lines.push(Line::raw(format!(" {}", fit(error, body_width))));
            lines.push(Line::styled(
                " Run `forge account list` in a shell to check.",
                theme.fg(Role::Muted),
            ));
        }
        (None, None) => lines.push(Line::styled(
            " Reading GitHub CLI accounts…",
            theme.fg(Role::Muted),
        )),
        (Some(accounts), None) if accounts.is_empty() => lines.push(Line::styled(
            " No accounts saved in GitHub CLI.",
            theme.fg(Role::Muted),
        )),
        (Some(accounts), None) => {
            for (index, account) in accounts.iter().enumerate() {
                let is_selected = index == picker.selected;
                let status = if picker.switching.as_deref() == Some(account.login.as_str()) {
                    Span::styled("switching…", theme.fg(Role::Mod))
                } else if account.active {
                    Span::styled("active", theme.fg(Role::Add))
                } else {
                    Span::raw("")
                };
                let mut line = Line::from(vec![
                    Span::styled(
                        if is_selected { " > " } else { "   " },
                        theme.fg(Role::Cursor),
                    ),
                    Span::styled(
                        if account.active { "● " } else { "  " },
                        theme.fg(Role::Add),
                    ),
                    Span::raw(fit(
                        &format!("@{}", account.login),
                        body_width.saturating_sub(18),
                    )),
                    Span::raw(" "),
                    status,
                ]);
                if is_selected {
                    line = line.style(theme.bg(Role::Selection));
                }
                lines.push(line);
            }
        }
    }
    lines.push(Line::raw(""));
    lines.push(Line::styled(
        " Add one: run `forge account add` in a shell.",
        theme.fg(Role::Subtle),
    ));
    frame.render_widget(Paragraph::new(lines), sections[0]);

    let hints = [
        hint("↑↓", "Move"),
        hint("enter", "Switch"),
        hint("esc", "Close"),
    ];
    let width = usize::from(sections[1].width);
    draw_bar(frame, theme, sections[1], key_line(theme, &hints, width));
}

fn draw_title(frame: &mut Frame<'_>, app: &ProjectBrowser, theme: &Theme, area: Rect) {
    let count = app.projects.len();
    let mut summary = match (app.loading, app.discovered) {
        (true, None) => "scanning…".to_owned(),
        (true, Some(total)) => format!(
            "reading {}/{total} · {} need attention",
            count + app.unreadable,
            app.needs_attention()
        ),
        (false, _) => format!(
            "{count} {} · {} need attention",
            if count == 1 {
                "repository"
            } else {
                "repositories"
            },
            app.needs_attention()
        ),
    };
    if app.unreadable > 0 {
        summary.push_str(&format!(" · {} unreadable", app.unreadable));
    }
    let context = vec![
        Span::styled(display_root(&app.root), theme.fg(Role::Ink)),
        Span::raw("  "),
        Span::styled(summary, theme.fg(Role::Muted)),
    ];
    draw_title_bar(frame, theme, area, context, app.user.as_deref());
}

fn draw_key_bar(frame: &mut Frame<'_>, app: &ProjectBrowser, theme: &Theme, area: Rect) {
    let line = if app.filtering {
        let key = |label: &'static str| Span::styled(label, theme.bold(Role::Forge));
        Line::from(vec![
            Span::raw(" "),
            key("/"),
            Span::styled(format!(" {}▏", app.filter), theme.fg(Role::Ink)),
            Span::raw("   "),
            key("enter"),
            Span::raw(" Keep   "),
            key("esc"),
            Span::raw(" Clear"),
        ])
    } else {
        let hints = [
            primary("enter", "Open"),
            hint("1–9", "Jump"),
            hint("/", "Filter"),
            hint("tab", "Scope"),
            hint("@", "Account"),
            hint("q", "Quit"),
        ];
        key_line(theme, &hints, usize::from(area.width))
    };
    draw_bar(frame, theme, area, line);
}

fn draw_project_list(
    frame: &mut Frame<'_>,
    app: &ProjectBrowser,
    theme: &Theme,
    area: Rect,
    visible: &[usize],
    with_detail: bool,
) {
    let block = Block::default()
        .borders(if with_detail {
            Borders::RIGHT
        } else {
            Borders::NONE
        })
        .border_style(theme.fg(Role::Line))
        .padding(Padding::new(1, 1, 1, 0));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(1),
        ])
        .split(inner);

    let mut prompt = vec![
        Span::styled("/ ", theme.fg(Role::Cursor)),
        if app.filter.is_empty() {
            Span::styled("filter projects…", theme.fg(Role::Subtle))
        } else {
            Span::styled(app.filter.clone(), theme.fg(Role::Ink))
        },
        Span::raw("  "),
    ];
    for scope in Scope::ORDER {
        prompt.push(if scope == app.scope {
            Span::styled(format!("[{}]", scope.label()), theme.bold(Role::Cursor))
        } else {
            Span::styled(format!(" {} ", scope.label()), theme.fg(Role::Muted))
        });
    }
    frame.render_widget(Paragraph::new(Line::from(prompt)), sections[0]);

    if app.loading && app.projects.is_empty() {
        frame.render_widget(
            Paragraph::new(format!(
                "Scanning local repositories in {}…",
                display_root(&app.root)
            ))
            .style(theme.fg(Role::Muted))
            .wrap(Wrap { trim: true }),
            sections[3],
        );
        return;
    }

    if let Some(error) = &app.error {
        frame.render_widget(
            Paragraph::new(format!(
                "Could not load local projects.\n\n{error}\n\nCheck the path and Git installation, then restart Forge."
            ))
            .style(theme.fg(Role::Del))
            .wrap(Wrap { trim: true }),
            sections[3],
        );
        return;
    }

    let columns = Columns::for_width(usize::from(inner.width));
    let mut header = format!("    {}", fit("project", columns.name));
    if let Some(width) = columns.branch {
        header.push(' ');
        header.push_str(&fit("branch", width));
    }
    header.push_str(" ↑ ↓     dirty ci last");
    frame.render_widget(
        Paragraph::new(Line::styled(header, theme.fg(Role::Subtle))),
        sections[2],
    );

    if visible.is_empty() {
        let message = if app.projects.is_empty() {
            format!(
                "No local Git repositories found in {}.\n\nSet {PROJECTS_DIR_VARIABLE} to the folder that holds your repositories, or start forge from that folder.",
                display_root(&app.root)
            )
        } else if app.filter.is_empty() {
            format!(
                "No projects are {}. Press tab to change the scope.",
                app.scope.label()
            )
        } else {
            "No projects match this filter. Press / then esc to clear it.".to_owned()
        };
        frame.render_widget(
            Paragraph::new(message)
                .style(theme.fg(Role::Muted))
                .wrap(Wrap { trim: true }),
            sections[3],
        );
        return;
    }

    let now = unix_now();
    let mut rows = Vec::new();
    let mut selected_row = None;
    let mut previous_group = "";
    for (position, index) in visible.iter().enumerate() {
        let project = &app.projects[*index];
        if project.group != previous_group {
            previous_group = &project.group;
            rows.push(ListItem::new(heading(theme, &project.group)));
        }
        let is_selected = *index == app.selected;
        if is_selected {
            selected_row = Some(rows.len());
        }
        rows.push(ListItem::new(project_row(
            theme,
            project,
            position,
            is_selected,
            &columns,
            now,
        )));
    }

    let mut state = ListState::default();
    state.select(selected_row);
    frame.render_stateful_widget(
        List::new(rows).highlight_style(theme.bg(Role::Selection)),
        sections[3],
        &mut state,
    );
}

fn project_row(
    theme: &Theme,
    project: &Project,
    position: usize,
    is_selected: bool,
    columns: &Columns,
    now: u64,
) -> Line<'static> {
    let mut name_style = theme.fg(Role::Ink);
    if is_selected {
        name_style = name_style.add_modifier(Modifier::BOLD);
    }
    let mut spans = vec![
        Span::styled(
            if is_selected { "> " } else { "  " },
            theme.fg(Role::Cursor),
        ),
        Span::styled(
            if position < 9 {
                format!("{} ", position + 1)
            } else {
                "  ".to_owned()
            },
            theme.bold(Role::Forge),
        ),
        Span::styled(fit(&project.name, columns.name), name_style),
    ];
    if let Some(width) = columns.branch {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            fit(&format!("⎇ {}", project.branch), width),
            theme.fg(Role::Muted),
        ));
    }

    spans.push(Span::raw(" "));
    if project.upstream {
        let ahead = format!("↑{}", project.ahead);
        let behind = format!("↓{}", project.behind);
        let pad = 7usize.saturating_sub(ahead.chars().count() + 1 + behind.chars().count());
        spans.push(Span::styled(ahead, theme.fg(Role::Add)));
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            behind,
            theme.fg(if project.behind > 0 {
                Role::Del
            } else {
                Role::Muted
            }),
        ));
        spans.push(Span::raw(" ".repeat(pad)));
    } else {
        spans.push(Span::styled(fit("—", 7), theme.fg(Role::Subtle)));
    }

    spans.push(Span::raw(" "));
    spans.push(if project.dirty_files > 0 {
        Span::styled(
            fit(&project.dirty_files.to_string(), 5),
            theme.fg(Role::Mod),
        )
    } else {
        Span::styled(fit("—", 5), theme.fg(Role::Subtle))
    });
    spans.push(Span::raw(" "));
    spans.push(Span::styled(fit("·", 2), theme.fg(Role::Subtle)));
    spans.push(Span::raw(" "));
    spans.push(Span::styled(
        project
            .last_commit
            .map_or_else(|| "—".to_owned(), |then| relative_age(now, then)),
        theme.fg(Role::Subtle),
    ));
    Line::from(spans)
}

fn draw_project_detail(
    frame: &mut Frame<'_>,
    app: &ProjectBrowser,
    theme: &Theme,
    area: Rect,
    visible: &[usize],
) {
    let block = Block::default().padding(Padding::new(2, 1, 1, 0));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let Some(project) = visible
        .iter()
        .find(|index| **index == app.selected)
        .map(|index| &app.projects[*index])
    else {
        if !app.loading {
            frame.render_widget(
                Paragraph::new("Select a project to see its local status and recent commits.")
                    .style(theme.fg(Role::Muted))
                    .wrap(Wrap { trim: true }),
                inner,
            );
        }
        return;
    };

    let label = |text: &'static str| Span::styled(text, theme.fg(Role::Muted));
    let (changes, changes_color) = match project.dirty_files {
        0 => ("clean".to_owned(), Role::Add),
        1 => ("1 file changed".to_owned(), Role::Mod),
        count => (format!("{count} files changed"), Role::Mod),
    };
    let width = usize::from(inner.width);
    let value_width = width.saturating_sub(8);
    let branch = if project.branch.is_empty() {
        "⎇ (unborn)".to_owned()
    } else {
        format!("⎇ {}", project.branch)
    };
    let mut lines = vec![
        heading(theme, &fit(&project.name, width)),
        Line::styled(
            fit(&display_root(&project.path).replace('\\', "/"), width),
            theme.fg(Role::Muted),
        ),
        Line::raw(""),
        Line::from(vec![
            label("branch  "),
            Span::raw(fit(&branch, value_width)),
        ]),
        Line::from(vec![
            label("remote  "),
            Span::raw(fit(
                project.remote.as_deref().unwrap_or("not configured"),
                value_width,
            )),
        ]),
        Line::from(vec![
            label("changes "),
            Span::styled(changes, theme.fg(changes_color)),
        ]),
        Line::from(vec![
            label("ci      "),
            Span::styled("· not fetched", theme.fg(Role::Subtle)),
        ]),
        Line::raw(""),
        heading(theme, "Recent commits"),
    ];
    if project.commits.is_empty() {
        lines.push(Line::styled("No commits yet.", theme.fg(Role::Muted)));
    } else {
        lines.extend(project.commits.iter().map(|commit| {
            Line::from(vec![
                Span::styled(commit.hash.clone(), theme.fg(Role::Mod)),
                Span::raw("  "),
                Span::raw(commit.subject.clone()),
            ])
        }));
    }
    lines.push(Line::styled("─".repeat(width), theme.fg(Role::Line)));
    lines.push(heading(theme, "Open with"));
    lines.push(Line::styled("[enter · Changes]", theme.bold(Role::Forge)));
    frame.render_widget(Paragraph::new(lines), inner);
}
