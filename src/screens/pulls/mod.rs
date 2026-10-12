//! 05 · Pull requests — browse GitHub pull requests, read their checks and reviews,
//! and check one out. Everything on this screen comes from GitHub.

mod view;

pub use view::draw_pulls;

use crate::{
    git::repository::BranchTracking,
    github::pulls::{
        PullDetail, PullFilter, PullRequest, checkout_pull, list_pulls, open_pull, read_pull,
    },
    screens::Screen,
    ui::{Notice, unix_now},
};
use crossterm::event::{KeyCode, KeyModifiers};
use std::{collections::HashMap, path::PathBuf, sync::mpsc, thread};

const PAGE_STEP: isize = 10;

/// Background work requested by the Pull requests screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PullsRequest {
    List(PullFilter),
    Detail(u64),
    Checkout(u64),
    Open(u64),
}

pub enum PullsEvent {
    Listed(PullFilter, Result<Vec<PullRequest>, String>),
    Detail(u64, Result<PullDetail, String>),
    Done(Result<String, String>),
}

/// Runs `request` on a worker thread. Results carry the repository path so a
/// screen opened later for another project can ignore them.
pub fn spawn_request(
    repo: PathBuf,
    request: PullsRequest,
    sender: mpsc::Sender<(PathBuf, PullsEvent)>,
) {
    thread::spawn(move || {
        let event = match request {
            PullsRequest::List(filter) => PullsEvent::Listed(filter, list_pulls(&repo, filter)),
            PullsRequest::Detail(number) => PullsEvent::Detail(number, read_pull(&repo, number)),
            PullsRequest::Checkout(number) => PullsEvent::Done(checkout_pull(&repo, number)),
            PullsRequest::Open(number) => PullsEvent::Done(open_pull(&repo, number)),
        };
        let _ = sender.send((repo, event));
    });
}

pub struct PullsView {
    pub path: PathBuf,
    tracking: Option<BranchTracking>,
    filter: PullFilter,
    pulls: Option<Vec<PullRequest>>,
    /// When GitHub last answered the list request (Unix seconds).
    fetched: Option<u64>,
    load_error: Option<String>,
    loading: bool,
    selected: usize,
    /// The pull request to put the cursor back on after a reload.
    keep: Option<u64>,
    /// Details already fetched in this visit, by pull request number.
    details: HashMap<u64, Result<PullDetail, String>>,
    /// On narrow terminals the detail replaces the list while this is set.
    detail_open: bool,
    busy: Option<&'static str>,
    notice: Option<Notice>,
    /// Background work for the run loop to start.
    pub request: Option<PullsRequest>,
    /// Esc or `1` was pressed: return to Changes.
    pub closed: bool,
    /// A tab key asked for another screen.
    pub goto: Option<Screen>,
    pub should_quit: bool,
}

impl PullsView {
    /// A view for `path` that asks the run loop to fetch the open pull requests.
    pub fn open(path: PathBuf, tracking: Option<BranchTracking>) -> Self {
        Self {
            path,
            tracking,
            filter: PullFilter::Open,
            pulls: None,
            fetched: None,
            load_error: None,
            loading: true,
            selected: 0,
            keep: None,
            details: HashMap::new(),
            detail_open: false,
            busy: None,
            notice: None,
            request: Some(PullsRequest::List(PullFilter::Open)),
            closed: false,
            goto: None,
            should_quit: false,
        }
    }

    fn list(&self) -> &[PullRequest] {
        self.pulls.as_deref().unwrap_or(&[])
    }

    fn selected_pull(&self) -> Option<&PullRequest> {
        self.list().get(self.selected)
    }

    fn reload(&mut self) {
        if self.keep.is_none() {
            self.keep = self.selected_pull().map(|pull| pull.number);
        }
        self.loading = true;
        self.details.clear();
        self.request = Some(PullsRequest::List(self.filter));
    }

    pub fn apply(&mut self, event: PullsEvent) {
        match event {
            PullsEvent::Listed(filter, result) => {
                // An answer for a filter the screen has since left is dropped.
                if filter != self.filter {
                    return;
                }
                self.loading = false;
                match result {
                    Ok(pulls) => {
                        self.load_error = None;
                        self.fetched = Some(unix_now());
                        let wanted = self.keep.take();
                        self.selected = wanted
                            .and_then(|number| pulls.iter().position(|pull| pull.number == number))
                            .unwrap_or_else(|| self.selected.min(pulls.len().saturating_sub(1)));
                        self.pulls = Some(pulls);
                        self.request_detail();
                    }
                    Err(error) => {
                        self.keep = None;
                        self.load_error = Some(error);
                    }
                }
            }
            PullsEvent::Detail(number, result) => {
                self.details.insert(number, result);
            }
            PullsEvent::Done(result) => {
                self.busy = None;
                self.notice = Some(match result {
                    Ok(text) => Notice::info(text),
                    Err(text) => Notice::error(text),
                });
                self.request_detail();
            }
        }
    }

    /// Asks for the selected pull request's detail unless it is already here.
    fn request_detail(&mut self) {
        let Some(number) = self.selected_pull().map(|pull| pull.number) else {
            return;
        };
        if self.details.contains_key(&number) {
            return;
        }
        // Never replace a reload, checkout or open that has not started yet.
        let pending = matches!(
            self.request,
            Some(PullsRequest::List(_) | PullsRequest::Checkout(_) | PullsRequest::Open(_))
        );
        if !pending {
            self.request = Some(PullsRequest::Detail(number));
        }
    }

    fn move_cursor(&mut self, delta: isize) {
        let count = self.list().len();
        if count > 0 {
            self.selected = (self.selected as isize + delta).clamp(0, count as isize - 1) as usize;
            self.request_detail();
        }
    }

    fn run(&mut self, request: PullsRequest, label: &'static str) {
        self.busy = Some(label);
        self.notice = None;
        self.request = Some(request);
    }

    pub fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        if modifiers.contains(KeyModifiers::CONTROL) {
            if code == KeyCode::Char('c') {
                self.should_quit = true;
            }
            return;
        }
        let mutates = matches!(code, KeyCode::Enter | KeyCode::Char('c'));
        if mutates && self.busy.is_some() {
            self.notice = Some(Notice::info("Still working on the last request…"));
            return;
        }
        match code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char(digit @ '1'..='9') => match Screen::from_digit(digit) {
                Some(Screen::Changes) => self.closed = true,
                Some(Screen::PullRequests) | None => {}
                Some(screen) => self.goto = Some(screen),
            },
            KeyCode::Esc if self.detail_open => self.detail_open = false,
            KeyCode::Esc => self.closed = true,
            KeyCode::Tab | KeyCode::BackTab => self.detail_open = !self.detail_open,
            KeyCode::Up | KeyCode::Char('k') => self.move_cursor(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_cursor(1),
            KeyCode::PageUp => self.move_cursor(-PAGE_STEP),
            KeyCode::PageDown => self.move_cursor(PAGE_STEP),
            KeyCode::Home | KeyCode::Char('g') => self.move_cursor(isize::MIN / 2),
            KeyCode::End | KeyCode::Char('G') => self.move_cursor(isize::MAX / 2),
            KeyCode::Enter => {
                if let Some(number) = self.selected_pull().map(|pull| pull.number) {
                    self.run(PullsRequest::Open(number), "opening…");
                }
            }
            KeyCode::Char('c') => {
                if let Some(number) = self.selected_pull().map(|pull| pull.number) {
                    self.run(PullsRequest::Checkout(number), "checking out…");
                }
            }
            KeyCode::Char('f') => {
                self.filter = self.filter.next();
                self.keep = None;
                self.selected = 0;
                self.pulls = None;
                self.reload();
            }
            KeyCode::Char('r') => {
                self.notice = None;
                self.reload();
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests;
