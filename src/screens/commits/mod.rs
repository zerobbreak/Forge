//! 03 · Commits — the history as a graph, with the selected commit's detail.

mod view;

pub use view::draw_commits;

use crate::git::{
    log::{CommitDetail, CommitInfo, LogPage, LogRow, LogScope, PAGE, read_commit_detail, read_log},
    repository::BranchTracking,
};
use crossterm::event::{KeyCode, KeyModifiers};
use std::{path::PathBuf, sync::mpsc, thread};

/// Commits moved by `PageUp`/`PageDown`.
const PAGE_STEP: usize = 10;
/// More history is fetched when the cursor is this close to the end of what is loaded.
const FETCH_AHEAD: usize = 10;

/// Background work requested by the Commits screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommitsRequest {
    Load { scope: LogScope, limit: usize },
    Detail(String),
}

pub enum CommitsEvent {
    Loaded {
        scope: LogScope,
        limit: usize,
        result: Result<LogPage, String>,
    },
    Detail(String, Result<CommitDetail, String>),
}

/// Runs `request` on a worker thread. Results carry the repository path so a
/// screen opened later for another project can ignore them.
pub fn spawn_request(
    repo: PathBuf,
    request: CommitsRequest,
    sender: mpsc::Sender<(PathBuf, CommitsEvent)>,
) {
    thread::spawn(move || {
        let event = match request {
            CommitsRequest::Load { scope, limit } => CommitsEvent::Loaded {
                scope,
                limit,
                result: read_log(&repo, scope, limit),
            },
            CommitsRequest::Detail(hash) => {
                let result = read_commit_detail(&repo, &hash);
                CommitsEvent::Detail(hash, result)
            }
        };
        let _ = sender.send((repo, event));
    });
}

pub struct CommitsView {
    pub path: PathBuf,
    tracking: Option<BranchTracking>,
    scope: LogScope,
    limit: usize,
    rows: Vec<LogRow>,
    /// Older commits exist beyond `limit`.
    more: bool,
    loading: bool,
    load_error: Option<String>,
    /// Index into `rows`; always a row that has a commit.
    selected: usize,
    /// The commit to put the cursor back on after a reload.
    keep: Option<String>,
    detail: Option<(String, Result<CommitDetail, String>)>,
    /// On narrow terminals the detail replaces the list while this is set.
    detail_open: bool,
    /// Background work for the run loop to start.
    pub request: Option<CommitsRequest>,
    /// Esc was pressed: return to Changes.
    pub closed: bool,
    pub should_quit: bool,
}

impl CommitsView {
    /// A view for `path` that asks the run loop to read the history.
    pub fn open(path: PathBuf, tracking: Option<BranchTracking>) -> Self {
        Self {
            path,
            tracking,
            scope: LogScope::All,
            limit: PAGE,
            rows: Vec::new(),
            more: false,
            loading: true,
            load_error: None,
            selected: 0,
            keep: None,
            detail: None,
            detail_open: false,
            request: Some(CommitsRequest::Load {
                scope: LogScope::All,
                limit: PAGE,
            }),
            closed: false,
            should_quit: false,
        }
    }

    fn commit_at(&self, index: usize) -> Option<&CommitInfo> {
        self.rows.get(index)?.commit.as_ref()
    }

    fn selected_commit(&self) -> Option<&CommitInfo> {
        self.commit_at(self.selected)
    }

    /// Commits loaded, not counting rows that only connect lanes.
    fn commit_count(&self) -> usize {
        self.rows.iter().filter(|row| row.commit.is_some()).count()
    }

    fn reload(&mut self) {
        self.keep = self.selected_commit().map(|commit| commit.hash.clone());
        self.loading = true;
        self.request = Some(CommitsRequest::Load {
            scope: self.scope,
            limit: self.limit,
        });
    }

    pub fn apply(&mut self, event: CommitsEvent) {
        match event {
            CommitsEvent::Loaded {
                scope,
                limit,
                result,
            } => {
                // An answer to a question the screen has since changed is dropped.
                if scope != self.scope || limit != self.limit {
                    return;
                }
                self.loading = false;
                match result {
                    Ok(page) => {
                        self.rows = page.rows;
                        self.more = page.more;
                        self.load_error = None;
                        let wanted = self.keep.take();
                        self.selected = wanted
                            .and_then(|hash| {
                                self.rows.iter().position(|row| {
                                    row.commit.as_ref().is_some_and(|c| c.hash == hash)
                                })
                            })
                            .or_else(|| self.rows.iter().position(|row| row.commit.is_some()))
                            .unwrap_or(0);
                        self.request_detail();
                    }
                    Err(error) => self.load_error = Some(error),
                }
            }
            CommitsEvent::Detail(hash, result) => {
                if self.selected_commit().is_some_and(|commit| commit.hash == hash) {
                    self.detail = Some((hash, result));
                }
            }
        }
    }

    fn request_detail(&mut self) {
        let Some(hash) = self.selected_commit().map(|commit| commit.hash.clone()) else {
            self.detail = None;
            return;
        };
        if self.detail.as_ref().is_some_and(|(loaded, _)| *loaded == hash) {
            return;
        }
        // A pending reload must not be replaced; it asks for the detail itself when it lands.
        if !matches!(self.request, Some(CommitsRequest::Load { .. })) {
            self.request = Some(CommitsRequest::Detail(hash));
        }
    }

    /// Moves the cursor `delta` commits, skipping rows that only draw lanes.
    fn move_cursor(&mut self, delta: isize) {
        let mut index = self.selected;
        let mut remaining = delta.unsigned_abs();
        while remaining > 0 {
            let next = if delta < 0 {
                index.checked_sub(1)
            } else {
                Some(index + 1).filter(|next| *next < self.rows.len())
            };
            let Some(next) = next else { break };
            index = next;
            if self.rows[index].commit.is_some() {
                remaining -= 1;
                self.selected = index;
            }
        }
        self.request_detail();
        self.fetch_older_when_near_the_end();
    }

    fn move_to_end(&mut self, last: bool) {
        let target = if last {
            self.rows.iter().rposition(|row| row.commit.is_some())
        } else {
            self.rows.iter().position(|row| row.commit.is_some())
        };
        if let Some(target) = target {
            self.selected = target;
            self.request_detail();
            self.fetch_older_when_near_the_end();
        }
    }

    fn fetch_older_when_near_the_end(&mut self) {
        if !self.more || self.loading {
            return;
        }
        let position = self
            .rows
            .iter()
            .take(self.selected + 1)
            .filter(|row| row.commit.is_some())
            .count();
        if self.commit_count().saturating_sub(position) <= FETCH_AHEAD {
            self.limit += PAGE;
            self.reload();
        }
    }

    fn toggle_scope(&mut self) {
        self.scope = match self.scope {
            LogScope::All => LogScope::Head,
            LogScope::Head => LogScope::All,
        };
        self.limit = PAGE;
        self.reload();
    }

    pub fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        if modifiers.contains(KeyModifiers::CONTROL) {
            if code == KeyCode::Char('c') {
                self.should_quit = true;
            }
            return;
        }
        match code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Esc if self.detail_open => self.detail_open = false,
            KeyCode::Esc => self.closed = true,
            KeyCode::Enter => self.detail_open = !self.detail_open,
            KeyCode::Up | KeyCode::Char('k') => self.move_cursor(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_cursor(1),
            KeyCode::PageUp => self.move_cursor(-(PAGE_STEP as isize)),
            KeyCode::PageDown => self.move_cursor(PAGE_STEP as isize),
            KeyCode::Home | KeyCode::Char('g') => self.move_to_end(false),
            KeyCode::End | KeyCode::Char('G') => self.move_to_end(true),
            KeyCode::Char('a') => self.toggle_scope(),
            KeyCode::Char('r') => self.reload(),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests;
