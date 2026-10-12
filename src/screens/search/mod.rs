//! 08 · Find in files — search the project, preview each match, and replace the ones
//! you choose. Replace is destructive, so it is confirmed first and every file is
//! snapshotted before it is edited.

mod view;

pub use view::draw_search;

use crate::{
    editor::TextArea,
    git::{
        repository::BranchTracking,
        search::{
            Context, Match, ReplaceReport, SearchOptions, SearchResult, build_matcher,
            read_context, replace_in_line, replace_matches, search,
        },
    },
    screens::Screen,
    ui::Notice,
};
use crossterm::event::{KeyCode, KeyModifiers};
use regex::Regex;
use std::{
    collections::{BTreeSet, HashSet},
    path::PathBuf,
    sync::mpsc,
    thread,
};

/// Queries shorter than this are not searched; one letter matches nearly every line.
const MIN_QUERY: usize = 2;
const PAGE_STEP: isize = 10;
/// Lines of context shown above and below the selected match.
pub const CONTEXT_RADIUS: usize = 4;
/// Replacements previewed in the confirmation dialog.
const CONFIRM_PREVIEW: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    Find,
    Replace,
    Results,
}

/// Background work requested by the Search screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SearchRequest {
    Search {
        id: u64,
        query: String,
        options: SearchOptions,
    },
    Context {
        path: String,
        line: usize,
    },
    Replace {
        query: String,
        replacement: String,
        options: SearchOptions,
        targets: Vec<Match>,
    },
}

pub enum SearchEvent {
    Searched {
        id: u64,
        result: Result<SearchResult, String>,
    },
    Context {
        path: String,
        line: usize,
        result: Result<Context, String>,
    },
    Replaced(Result<ReplaceReport, String>),
}

/// Runs `request` on a worker thread. Results carry the repository path so a
/// screen opened later for another project can ignore them.
pub fn spawn_request(
    repo: PathBuf,
    request: SearchRequest,
    sender: mpsc::Sender<(PathBuf, SearchEvent)>,
) {
    thread::spawn(move || {
        let event = match request {
            SearchRequest::Search { id, query, options } => SearchEvent::Searched {
                id,
                result: search(&repo, &query, &options),
            },
            SearchRequest::Context { path, line } => SearchEvent::Context {
                result: read_context(&repo, &path, line, CONTEXT_RADIUS),
                path,
                line,
            },
            SearchRequest::Replace {
                query,
                replacement,
                options,
                targets,
            } => SearchEvent::Replaced(replace_matches(
                &repo,
                &query,
                &replacement,
                &options,
                &targets,
            )),
        };
        let _ = sender.send((repo, event));
    });
}

/// The pending replacement shown in the confirmation dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Confirm {
    targets: Vec<Match>,
    files: usize,
    /// `(path:line, old text, new text)` for the first few.
    preview: Vec<(String, String, String)>,
}

pub struct SearchView {
    pub path: PathBuf,
    tracking: Option<BranchTracking>,
    find: TextArea,
    replace: TextArea,
    options: SearchOptions,
    focus: Focus,
    /// The matcher for the text in the Find box, or why it is not valid.
    matcher: Result<Regex, String>,
    matches: Vec<Match>,
    /// The query and options the matches above answer.
    answered: Option<(String, SearchOptions)>,
    truncated: bool,
    search_error: Option<String>,
    /// Latest search id; answers with another id are dropped.
    sequence: u64,
    searching: bool,
    /// Index into `matches`.
    selected: usize,
    /// Matches left out of the replacement, as `(path, line)`.
    excluded: HashSet<(String, usize)>,
    context: Option<(String, usize, Result<Context, String>)>,
    confirm: Option<Confirm>,
    busy: Option<&'static str>,
    notice: Option<Notice>,
    /// Background work for the run loop to start.
    pub request: Option<SearchRequest>,
    /// Esc, or `1` in the results: return to Changes.
    pub closed: bool,
    /// A tab key asked for another screen.
    pub goto: Option<Screen>,
    /// `/` was pressed in the results: the app opens the command palette.
    pub open_palette: bool,
    pub should_quit: bool,
}

impl SearchView {
    pub fn open(path: PathBuf, tracking: Option<BranchTracking>) -> Self {
        Self {
            path,
            tracking,
            find: TextArea::single_line(""),
            replace: TextArea::single_line(""),
            options: SearchOptions::default(),
            focus: Focus::Find,
            matcher: build_matcher("", &SearchOptions::default()),
            matches: Vec::new(),
            answered: None,
            truncated: false,
            search_error: None,
            sequence: 0,
            searching: false,
            selected: 0,
            excluded: HashSet::new(),
            context: None,
            confirm: None,
            busy: None,
            notice: None,
            request: None,
            closed: false,
            goto: None,
            open_palette: false,
            should_quit: false,
        }
    }

    fn query(&self) -> String {
        self.find.text()
    }

    fn replacement(&self) -> String {
        self.replace.text()
    }

    fn selected_match(&self) -> Option<&Match> {
        self.matches.get(self.selected)
    }

    fn is_included(&self, found: &Match) -> bool {
        !self.excluded.contains(&(found.path.clone(), found.line))
    }

    fn included(&self) -> Vec<Match> {
        self.matches
            .iter()
            .filter(|found| self.is_included(found))
            .cloned()
            .collect()
    }

    fn file_count(matches: &[Match]) -> usize {
        matches
            .iter()
            .map(|found| found.path.as_str())
            .collect::<BTreeSet<_>>()
            .len()
    }

    /// The results answer what is typed now, so replacing them is safe.
    fn results_are_current(&self) -> bool {
        !self.searching
            && self
                .answered
                .as_ref()
                .is_some_and(|(query, options)| *query == self.query() && *options == self.options)
    }

    /// Called whenever the Find text or an option changes.
    fn query_changed(&mut self) {
        self.matcher = build_matcher(&self.query(), &self.options);
        self.notice = None;
        if self.query().chars().count() < MIN_QUERY || self.matcher.is_err() {
            self.matches.clear();
            self.answered = None;
            self.truncated = false;
            self.search_error = None;
            self.searching = false;
            self.context = None;
            self.selected = 0;
            self.sequence += 1;
            if matches!(self.request, Some(SearchRequest::Search { .. })) {
                self.request = None;
            }
            return;
        }
        self.start_search();
    }

    fn start_search(&mut self) {
        // A replace in flight owns the request slot; its answer starts a fresh search.
        if matches!(self.request, Some(SearchRequest::Replace { .. })) || self.busy.is_some() {
            return;
        }
        self.sequence += 1;
        self.searching = true;
        self.request = Some(SearchRequest::Search {
            id: self.sequence,
            query: self.query(),
            options: self.options,
        });
    }

    pub fn apply(&mut self, event: SearchEvent) {
        match event {
            SearchEvent::Searched { id, result } => {
                if id != self.sequence {
                    return;
                }
                self.searching = false;
                match result {
                    Ok(found) => self.show_results(found),
                    Err(error) => {
                        self.search_error = Some(error);
                        self.matches.clear();
                        self.answered = None;
                    }
                }
            }
            SearchEvent::Context { path, line, result } => {
                if self
                    .selected_match()
                    .is_some_and(|found| found.path == path && found.line == line)
                {
                    self.context = Some((path, line, result));
                }
            }
            SearchEvent::Replaced(result) => {
                self.busy = None;
                self.notice = Some(match result {
                    Ok(report) => replaced_notice(&report),
                    Err(error) => Notice::error(format!("Nothing more was replaced. {error}")),
                });
                self.excluded.clear();
                self.answered = None;
                self.context = None;
                self.start_search();
            }
        }
    }

    fn show_results(&mut self, found: SearchResult) {
        let same_query = self
            .answered
            .as_ref()
            .is_some_and(|(query, options)| *query == self.query() && *options == self.options);
        let previous = self
            .selected_match()
            .map(|found| (found.path.clone(), found.line));
        self.search_error = None;
        self.truncated = found.truncated;
        self.matches = found.matches;
        self.answered = Some((self.query(), self.options));
        if same_query {
            let still_there: HashSet<(String, usize)> = self
                .matches
                .iter()
                .map(|found| (found.path.clone(), found.line))
                .collect();
            self.excluded.retain(|key| still_there.contains(key));
        } else {
            self.excluded.clear();
        }
        self.selected = previous
            .and_then(|(path, line)| {
                self.matches
                    .iter()
                    .position(|found| found.path == path && found.line == line)
            })
            .unwrap_or(0);
        self.context = None;
        self.request_context();
    }

    fn request_context(&mut self) {
        let Some(found) = self.selected_match() else {
            return;
        };
        if self
            .context
            .as_ref()
            .is_some_and(|(path, line, _)| *path == found.path && *line == found.line)
        {
            return;
        }
        // Never replace a search or a replace that has not started yet.
        if self.request.is_none() || matches!(self.request, Some(SearchRequest::Context { .. })) {
            self.request = Some(SearchRequest::Context {
                path: found.path.clone(),
                line: found.line,
            });
        }
    }

    fn move_cursor(&mut self, delta: isize) {
        let count = self.matches.len();
        if count > 0 {
            self.selected = (self.selected as isize + delta).clamp(0, count as isize - 1) as usize;
            self.request_context();
        }
    }

    fn toggle_option(&mut self, option: OptionFlag) {
        let flag = option(&mut self.options);
        *flag = !*flag;
        self.query_changed();
    }

    fn toggle_selected(&mut self) {
        let Some(found) = self.selected_match() else {
            return;
        };
        let key = (found.path.clone(), found.line);
        if !self.excluded.remove(&key) {
            self.excluded.insert(key);
        }
    }

    /// Leaves out every match in the selected file, or puts them all back if they were all out.
    fn toggle_file(&mut self) {
        let Some(path) = self.selected_match().map(|found| found.path.clone()) else {
            return;
        };
        let keys: Vec<(String, usize)> = self
            .matches
            .iter()
            .filter(|found| found.path == path)
            .map(|found| (found.path.clone(), found.line))
            .collect();
        if keys.iter().all(|key| self.excluded.contains(key)) {
            for key in &keys {
                self.excluded.remove(key);
            }
        } else {
            self.excluded.extend(keys);
        }
    }

    fn say(&mut self, text: impl Into<String>) {
        self.notice = Some(Notice::info(text));
    }

    /// Opens the confirmation dialog for `targets`, or explains why not.
    fn ask_to_replace(&mut self, targets: Vec<Match>) {
        if self.replacement().is_empty() {
            return self
                .say("Type what to replace it with first (press Tab to reach the Replace box).");
        }
        if !self.results_are_current() {
            return self
                .say("The results are still catching up with your search. Try again in a moment.");
        }
        if targets.is_empty() {
            return self.say("Every match is left out. Press Space on a match to include it.");
        }
        let (Ok(matcher), replacement) = (&self.matcher, self.replacement()) else {
            return;
        };
        let preview = targets
            .iter()
            .take(CONFIRM_PREVIEW)
            .map(|found| {
                (
                    format!("{}:{}", found.path, found.line),
                    found.text.trim().to_owned(),
                    replace_in_line(matcher, &found.text, &replacement, &self.options)
                        .trim()
                        .to_owned(),
                )
            })
            .collect();
        self.confirm = Some(Confirm {
            files: Self::file_count(&targets),
            targets,
            preview,
        });
    }

    fn confirm_replace(&mut self) {
        let Some(confirm) = self.confirm.take() else {
            return;
        };
        self.busy = Some("replacing…");
        self.notice = None;
        self.request = Some(SearchRequest::Replace {
            query: self.query(),
            replacement: self.replacement(),
            options: self.options,
            targets: confirm.targets,
        });
    }

    pub fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        let control = modifiers.contains(KeyModifiers::CONTROL);
        let alt = modifiers.contains(KeyModifiers::ALT) && !control;
        if control && code == KeyCode::Char('c') {
            self.should_quit = true;
            return;
        }
        if self.confirm.is_some() {
            match code {
                KeyCode::Char('y' | 'Y') => self.confirm_replace(),
                KeyCode::Esc | KeyCode::Char('n' | 'N') => self.confirm = None,
                _ => {}
            }
            return;
        }
        if self.busy.is_some() && matches!(code, KeyCode::Enter | KeyCode::Char('R')) {
            return self.say("Still replacing…");
        }
        // Option toggles work from every box with Alt, and from the results with a letter.
        if let KeyCode::Char(letter) = code
            && (alt || (self.focus == Focus::Results && !control))
        {
            let option = match letter {
                'c' => Some(case_flag as OptionFlag),
                'e' => Some(regex_flag as OptionFlag),
                'w' => Some(word_flag as OptionFlag),
                _ => None,
            };
            if let Some(option) = option {
                self.toggle_option(option);
                return;
            }
        }
        match self.focus {
            Focus::Find | Focus::Replace => self.handle_input_key(code, modifiers),
            Focus::Results => self.handle_results_key(code),
        }
    }

    fn handle_input_key(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        match code {
            KeyCode::Esc => self.closed = true,
            KeyCode::Tab => {
                self.focus = match self.focus {
                    Focus::Find => Focus::Replace,
                    _ => Focus::Results,
                };
            }
            KeyCode::BackTab => {
                self.focus = match self.focus {
                    Focus::Replace => Focus::Find,
                    _ => Focus::Results,
                };
            }
            KeyCode::Enter | KeyCode::Down => {
                // Typing already searches; only start one if nothing has answered yet.
                if self.focus == Focus::Find
                    && code == KeyCode::Enter
                    && self.answered.is_none()
                    && !self.searching
                {
                    self.query_changed();
                }
                self.focus = Focus::Results;
            }
            _ => {
                let field = if self.focus == Focus::Find {
                    &mut self.find
                } else {
                    &mut self.replace
                };
                if field.handle_key(code, modifiers) && self.focus == Focus::Find {
                    self.query_changed();
                } else if self.focus == Focus::Replace {
                    self.notice = None;
                }
            }
        }
    }

    fn handle_results_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('/') => self.open_palette = true,
            KeyCode::Char(digit @ '1'..='9') => match Screen::from_digit(digit) {
                Some(Screen::Changes) => self.closed = true,
                Some(Screen::Search) | None => {}
                Some(screen) => self.goto = Some(screen),
            },
            KeyCode::Esc => self.closed = true,
            KeyCode::Tab | KeyCode::Char('i') => self.focus = Focus::Find,
            KeyCode::BackTab => self.focus = Focus::Replace,
            KeyCode::Up | KeyCode::Char('k') => self.move_cursor(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_cursor(1),
            KeyCode::PageUp => self.move_cursor(-PAGE_STEP),
            KeyCode::PageDown => self.move_cursor(PAGE_STEP),
            KeyCode::Home | KeyCode::Char('g') => self.move_cursor(isize::MIN / 2),
            KeyCode::End | KeyCode::Char('G') => self.move_cursor(isize::MAX / 2),
            KeyCode::Char(' ') => self.toggle_selected(),
            KeyCode::Char('x') => self.toggle_file(),
            KeyCode::Enter => {
                let target: Vec<Match> = self.selected_match().cloned().into_iter().collect();
                self.ask_to_replace(target);
            }
            KeyCode::Char('R') => {
                let targets = self.included();
                self.ask_to_replace(targets);
            }
            KeyCode::Char('r') => {
                self.notice = None;
                self.query_changed();
            }
            _ => {}
        }
    }
}

type OptionFlag = fn(&mut SearchOptions) -> &mut bool;

fn case_flag(options: &mut SearchOptions) -> &mut bool {
    &mut options.case_sensitive
}

fn regex_flag(options: &mut SearchOptions) -> &mut bool {
    &mut options.regex
}

fn word_flag(options: &mut SearchOptions) -> &mut bool {
    &mut options.whole_word
}

fn replaced_notice(report: &ReplaceReport) -> Notice {
    let skipped = report.changed_since_search + report.skipped_files.len();
    let skipped = if skipped > 0 {
        format!(" {skipped} left alone because the file changed or could not be edited safely.")
    } else {
        String::new()
    };
    let Some((path, snapshot)) = report.snapshots.first() else {
        return Notice::error(format!("Nothing was replaced.{skipped}"));
    };
    let more = if report.snapshots.len() > 1 {
        format!(
            " (+{} more under refs/forge/snapshots)",
            report.snapshots.len() - 1
        )
    } else {
        String::new()
    };
    Notice::info(format!(
        "Replaced {} line{} in {} file{}.{skipped} Undo: git checkout {snapshot} -- {path}{more}",
        report.lines,
        if report.lines == 1 { "" } else { "s" },
        report.files,
        if report.files == 1 { "" } else { "s" },
    ))
}

#[cfg(test)]
mod tests;
