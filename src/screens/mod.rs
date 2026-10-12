//! One module per TUI screen from the Forge design concept.

/// The repository screens reachable from each other with the tab strip and `1`–`5`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    Changes,
    Commits,
    Branches,
    PullRequests,
    Search,
}

impl Screen {
    /// Tabs in order; the position + 1 is the number key that opens the screen.
    pub const TABS: [(Screen, &'static str); 5] = [
        (Screen::Changes, "Changes"),
        (Screen::Commits, "Commits"),
        (Screen::Branches, "Branches"),
        (Screen::PullRequests, "Pull Requests"),
        (Screen::Search, "Search"),
    ];

    /// The screen a number key opens.
    pub fn from_digit(digit: char) -> Option<Self> {
        let index = digit.to_digit(10)?.checked_sub(1)?;
        Self::TABS.get(index as usize).map(|(screen, _)| *screen)
    }
}

pub mod branches;
pub mod changes;
pub mod commit;
pub mod commits;
pub mod palette;
pub mod projects;
pub mod pulls;
pub mod search;
