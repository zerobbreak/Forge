//! 07 · `/` command palette — find any command on the current screen by typing,
//! and see the key that runs it.
//!
//! A command is either a jump to another screen or one of the screen's own keys,
//! so the palette can never do something the keyboard cannot, and it teaches the keys.

mod view;

pub use view::draw_palette;

use crate::{editor::TextArea, screens::Screen};
use crossterm::event::{KeyCode, KeyModifiers};

/// What running a command does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Switch to another repository screen.
    Screen(Screen),
    /// Press one of the current screen's keys.
    Key(KeyCode),
    /// Leave the project and return to the project list.
    Projects,
    Quit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Command {
    pub title: &'static str,
    /// The shortcut shown beside the title.
    pub keys: &'static str,
    /// Extra words the query can match, e.g. "checkout" for "Check out".
    pub words: &'static str,
    pub action: Action,
}

const fn key(
    title: &'static str,
    keys: &'static str,
    code: KeyCode,
    words: &'static str,
) -> Command {
    Command {
        title,
        keys,
        words,
        action: Action::Key(code),
    }
}

const fn go(
    title: &'static str,
    keys: &'static str,
    screen: Screen,
    words: &'static str,
) -> Command {
    Command {
        title,
        keys,
        words,
        action: Action::Screen(screen),
    }
}

/// The commands for `screen`: its own first, then the ones every screen has.
pub fn commands(screen: Screen) -> Vec<Command> {
    let own: &[Command] = match screen {
        Screen::Changes => &[
            key("Stage hunk", "s", KeyCode::Char('s'), "add index"),
            key("Stage file", "a", KeyCode::Char('a'), "add index"),
            key("Unstage", "u", KeyCode::Char('u'), "remove index reset"),
            key(
                "Discard changes…",
                "x",
                KeyCode::Char('x'),
                "revert restore delete checkout",
            ),
            key("Commit…", "c", KeyCode::Char('c'), "message save"),
            key("Refresh", "r", KeyCode::Char('r'), "reload status"),
            key("Switch pane", "tab", KeyCode::Tab, "focus diff files"),
            key("Focus the sidebar", "h", KeyCode::Char('h'), "navigation"),
        ],
        Screen::Commits => &[
            key(
                "Toggle all branches / current branch",
                "a",
                KeyCode::Char('a'),
                "scope history",
            ),
            key("Newest commit", "g", KeyCode::Char('g'), "top first"),
            key(
                "Oldest loaded commit",
                "G",
                KeyCode::Char('G'),
                "bottom last end",
            ),
            key(
                "Show or hide details",
                "enter",
                KeyCode::Enter,
                "commit message files",
            ),
            key("Refresh", "r", KeyCode::Char('r'), "reload log history"),
        ],
        Screen::Branches => &[
            key(
                "Switch to the selected branch",
                "enter",
                KeyCode::Enter,
                "checkout change",
            ),
            key(
                "New branch from the selected one…",
                "n",
                KeyCode::Char('n'),
                "create",
            ),
            key("Rename branch…", "R", KeyCode::Char('R'), "move"),
            key("Delete branch…", "x", KeyCode::Char('x'), "remove"),
            key(
                "Show or hide details",
                "tab",
                KeyCode::Tab,
                "worktrees upstream",
            ),
            key("Refresh", "r", KeyCode::Char('r'), "reload"),
        ],
        Screen::PullRequests => &[
            key(
                "Open on GitHub",
                "enter",
                KeyCode::Enter,
                "browser web view",
            ),
            key(
                "Check out this pull request",
                "c",
                KeyCode::Char('c'),
                "checkout branch test",
            ),
            key(
                "Next state: open / merged / closed / all",
                "f",
                KeyCode::Char('f'),
                "filter",
            ),
            key(
                "Show or hide details",
                "tab",
                KeyCode::Tab,
                "checks reviews files",
            ),
            key(
                "Refresh from GitHub",
                "r",
                KeyCode::Char('r'),
                "reload fetch",
            ),
        ],
        Screen::Search => &[
            key(
                "Edit the search",
                "tab",
                KeyCode::Tab,
                "find query type text",
            ),
            key(
                "Leave this match out / put it back",
                "space",
                KeyCode::Char(' '),
                "skip exclude include select",
            ),
            key(
                "Leave this whole file out",
                "x",
                KeyCode::Char('x'),
                "skip exclude include",
            ),
            key(
                "Replace this match…",
                "enter",
                KeyCode::Enter,
                "substitute change",
            ),
            key(
                "Replace all included matches…",
                "R",
                KeyCode::Char('R'),
                "substitute change everything",
            ),
            key(
                "Match case",
                "c",
                KeyCode::Char('c'),
                "uppercase lowercase sensitive",
            ),
            key(
                "Regular expression",
                "e",
                KeyCode::Char('e'),
                "regex pattern",
            ),
            key("Whole word", "w", KeyCode::Char('w'), "exact"),
            key("Search again", "r", KeyCode::Char('r'), "refresh reload"),
        ],
    };
    let mut commands = own.to_vec();
    commands.extend([
        go(
            "Go to Changes",
            "1",
            Screen::Changes,
            "stage working tree status",
        ),
        go("Go to Commits", "2", Screen::Commits, "log history graph"),
        go("Go to Branches", "3", Screen::Branches, "worktrees switch"),
        go(
            "Go to Pull Requests",
            "4",
            Screen::PullRequests,
            "pr github review",
        ),
        go(
            "Go to Search",
            "5",
            Screen::Search,
            "find grep replace text files",
        ),
        Command {
            title: "Back to the project list",
            keys: "esc",
            words: "projects close home",
            action: Action::Projects,
        },
        Command {
            title: "Quit Forge",
            keys: "q",
            words: "exit close",
            action: Action::Quit,
        },
    ]);
    commands
}

/// What the palette wants the app to do next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Close,
    Run(Action),
}

pub struct PaletteView {
    /// The screen the palette was opened on; its commands are listed first.
    pub screen: Screen,
    query: TextArea,
    items: Vec<Command>,
    /// Indices into `items` that match the query, best first.
    matches: Vec<usize>,
    selected: usize,
    pub outcome: Option<Outcome>,
}

impl PaletteView {
    pub fn open(screen: Screen) -> Self {
        let items = commands(screen);
        let matches = (0..items.len()).collect();
        Self {
            screen,
            query: TextArea::single_line(""),
            items,
            matches,
            selected: 0,
            outcome: None,
        }
    }

    pub fn query(&self) -> String {
        self.query.text()
    }

    fn matching(&self) -> Vec<&Command> {
        self.matches
            .iter()
            .map(|index| &self.items[*index])
            .collect()
    }

    /// Re-ranks the commands for the current query. Every word typed must appear in a
    /// command's title or keywords; words found earlier in the title rank higher.
    fn refilter(&mut self) {
        let query = self.query.text().to_lowercase();
        let words: Vec<&str> = query.split_whitespace().collect();
        let mut ranked: Vec<(usize, usize)> = self
            .items
            .iter()
            .enumerate()
            .filter_map(|(index, command)| {
                let title = command.title.to_lowercase();
                let haystack = format!("{title} {}", command.words);
                let mut score = 0;
                for word in &words {
                    let position = haystack.find(word)?;
                    // Matches inside the title beat matches only in the keywords, and a
                    // match at the start of a word beats one in the middle.
                    let at_word_start = position == 0 || haystack.as_bytes()[position - 1] == b' ';
                    score += position + if at_word_start { 0 } else { 50 };
                    if position >= title.len() {
                        score += 100;
                    }
                }
                Some((score, index))
            })
            .collect();
        // Stable: commands with equal scores keep their listed order.
        ranked.sort_by_key(|(score, _)| *score);
        self.matches = ranked.into_iter().map(|(_, index)| index).collect();
        self.selected = 0;
    }

    pub fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        let control = modifiers.contains(KeyModifiers::CONTROL);
        match code {
            KeyCode::Esc => self.outcome = Some(Outcome::Close),
            KeyCode::Char('c') if control => self.outcome = Some(Outcome::Close),
            KeyCode::Enter => {
                self.outcome = self
                    .matching()
                    .get(self.selected)
                    .map(|command| Outcome::Run(command.action));
            }
            KeyCode::Up | KeyCode::BackTab => {
                self.selected = self.selected.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Tab => {
                if self.selected + 1 < self.matches.len() {
                    self.selected += 1;
                }
            }
            KeyCode::PageUp => self.selected = self.selected.saturating_sub(5),
            KeyCode::PageDown => {
                self.selected = (self.selected + 5).min(self.matches.len().saturating_sub(1));
            }
            _ => {
                if self.query.handle_key(code, modifiers) {
                    self.refilter();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
