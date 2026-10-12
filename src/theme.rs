//! Forge design-system colours. Every TUI surface styles through [`Theme`], so
//! `NO_COLOR` and palette changes apply everywhere at once.

use ratatui::style::{Color, Modifier, Style};
use std::env;

/// A named colour slot from the Forge design tokens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Panel,
    Raised,
    Selection,
    Line,
    LineStrong,
    Ink,
    Muted,
    Subtle,
    Forge,
    OnForge,
    Cursor,
    Add,
    AddBg,
    Del,
    DelBg,
    Mod,
}

/// One complete set of token colours.
#[derive(Clone, Copy, Debug)]
pub struct Palette {
    panel: Color,
    raised: Color,
    selection: Color,
    line: Color,
    line_strong: Color,
    ink: Color,
    muted: Color,
    subtle: Color,
    forge: Color,
    on_forge: Color,
    cursor: Color,
    add: Color,
    add_bg: Color,
    del: Color,
    del_bg: Color,
    modified: Color,
}

impl Palette {
    /// The default Forge dark theme.
    pub const DARK: Self = Self {
        panel: Color::Rgb(17, 22, 29),
        raised: Color::Rgb(24, 32, 42),
        selection: Color::Rgb(28, 53, 64),
        line: Color::Rgb(42, 52, 65),
        line_strong: Color::Rgb(86, 104, 124),
        ink: Color::Rgb(220, 227, 234),
        muted: Color::Rgb(149, 162, 176),
        subtle: Color::Rgb(125, 138, 152),
        forge: Color::Rgb(255, 158, 79),
        on_forge: Color::Rgb(26, 15, 5),
        cursor: Color::Rgb(86, 199, 214),
        add: Color::Rgb(126, 211, 139),
        add_bg: Color::Rgb(19, 42, 27),
        del: Color::Rgb(242, 129, 122),
        del_bg: Color::Rgb(48, 24, 24),
        modified: Color::Rgb(233, 196, 106),
    };

    fn color(&self, role: Role) -> Color {
        match role {
            Role::Panel => self.panel,
            Role::Raised => self.raised,
            Role::Selection => self.selection,
            Role::Line => self.line,
            Role::LineStrong => self.line_strong,
            Role::Ink => self.ink,
            Role::Muted => self.muted,
            Role::Subtle => self.subtle,
            Role::Forge => self.forge,
            Role::OnForge => self.on_forge,
            Role::Cursor => self.cursor,
            Role::Add => self.add,
            Role::AddBg => self.add_bg,
            Role::Del => self.del,
            Role::DelBg => self.del_bg,
            Role::Mod => self.modified,
        }
    }
}

/// The active palette plus whether colour output is allowed.
#[derive(Clone, Copy, Debug)]
pub struct Theme {
    palette: Palette,
    colors: bool,
}

impl Theme {
    /// Dark palette, with colour disabled when `NO_COLOR` is set.
    pub fn from_env() -> Self {
        Self::new(env::var_os("NO_COLOR").is_none())
    }

    pub fn new(colors: bool) -> Self {
        Self {
            palette: Palette::DARK,
            colors,
        }
    }

    /// Foreground in `role`, or the terminal default when colour is off.
    pub fn fg(&self, role: Role) -> Style {
        if self.colors {
            Style::default().fg(self.palette.color(role))
        } else {
            Style::default()
        }
    }

    /// Background in `role`, or the terminal default when colour is off.
    pub fn bg(&self, role: Role) -> Style {
        if self.colors {
            Style::default().bg(self.palette.color(role))
        } else {
            Style::default()
        }
    }

    pub fn bold(&self, role: Role) -> Style {
        self.fg(role).add_modifier(Modifier::BOLD)
    }

    /// The filled orange key used for a screen's primary action.
    pub fn primary(&self) -> Style {
        if self.colors {
            Style::default()
                .fg(self.palette.color(Role::OnForge))
                .bg(self.palette.color(Role::Forge))
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().add_modifier(Modifier::BOLD | Modifier::REVERSED)
        }
    }
}
