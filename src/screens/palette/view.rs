use super::PaletteView;
use crate::{
    screens::Screen,
    theme::{Role, Theme},
    ui::{draw_bar, fit, hint, key_line, popup, primary},
};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Position},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, ListState, Padding, Paragraph},
};

const WIDTH: u16 = 70;
/// Rows of commands shown at once.
const ROWS: u16 = 10;
const KEYS_WIDTH: usize = 6;

fn screen_name(screen: Screen) -> &'static str {
    Screen::TABS
        .iter()
        .find(|(tab, _)| *tab == screen)
        .map_or("", |(_, name)| name)
}

pub fn draw_palette(frame: &mut Frame<'_>, view: &PaletteView, theme: &Theme) {
    let matches = view.matching();
    let rows = u16::try_from(matches.len().max(1))
        .unwrap_or(ROWS)
        .min(ROWS);
    // Input, rule, command rows, hint row, plus the border.
    let popup = popup(frame.area(), WIDTH, rows + 6);
    frame.render_widget(Clear, popup);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme.fg(Role::Forge))
        .title(Span::styled(
            format!(" COMMANDS · {} ", screen_name(view.screen).to_uppercase()),
            theme.bold(Role::Forge),
        ))
        .padding(Padding::horizontal(1))
        .style(theme.bg(Role::Panel).patch(theme.fg(Role::Ink)));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    let parts = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(inner);

    // The query, with the cursor after it; long queries scroll.
    let query = view.query();
    let room = usize::from(parts[0].width).saturating_sub(3).max(1);
    let characters = query.chars().count();
    let start = characters.saturating_sub(room);
    let shown: String = query.chars().skip(start).collect();
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("/ ", theme.bold(Role::Forge)),
            Span::styled(shown, theme.bold(Role::Ink)),
        ])),
        parts[0],
    );
    frame.set_cursor_position(Position::new(
        parts[0].x + 2 + (characters - start) as u16,
        parts[0].y,
    ));
    frame.render_widget(
        Paragraph::new(Line::styled(
            "─".repeat(usize::from(parts[1].width)),
            theme.fg(Role::Line),
        )),
        parts[1],
    );

    if matches.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::styled(
                format!("No command matches “{query}”. Clear the text to see them all."),
                theme.fg(Role::Muted),
            )),
            parts[2],
        );
    } else {
        let width = usize::from(parts[2].width);
        let title_width = width.saturating_sub(2 + KEYS_WIDTH + 1);
        let items: Vec<ListItem> = matches
            .iter()
            .enumerate()
            .map(|(index, command)| {
                let selected = index == view.selected;
                let line = Line::from(vec![
                    if selected {
                        Span::styled("> ", theme.fg(Role::Cursor))
                    } else {
                        Span::raw("  ")
                    },
                    Span::styled(
                        fit(command.title, title_width),
                        if selected {
                            theme.bold(Role::Cursor)
                        } else {
                            theme.fg(Role::Ink)
                        },
                    ),
                    Span::styled(
                        format!(" {:>KEYS_WIDTH$}", command.keys),
                        theme.fg(Role::Forge),
                    ),
                ]);
                ListItem::new(if selected {
                    line.style(theme.bg(Role::Selection))
                } else {
                    line
                })
            })
            .collect();
        let mut state = ListState::default().with_selected(Some(view.selected));
        frame.render_stateful_widget(List::new(items), parts[2], &mut state);
    }

    let width = usize::from(parts[3].width);
    draw_bar(
        frame,
        theme,
        parts[3],
        key_line(
            theme,
            &[
                primary("enter", "Run"),
                hint("↑↓", "Choose"),
                hint("esc", "Close"),
            ],
            width,
        ),
    );
}
