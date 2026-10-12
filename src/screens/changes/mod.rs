//! 01 · Changes — review the working tree and stage it by file or by hunk.

mod view;

pub use view::{draw_changes, file_row};

use crate::git::changes::{
    DiffLine, FileChange, FileDiff, FileState, WorkingTree, apply_to_index, diff_for, discard_file,
    discard_hunk, read_working_tree, stage_file, unstage_file,
};
use crate::{git::repository::BranchTracking, ui::Notice};
use crossterm::event::{KeyCode, KeyModifiers};
use std::{path::PathBuf, sync::mpsc, thread};

/// Rows moved by `PageUp`/`PageDown` (and `ctrl+u`/`ctrl+d`) in the diff.
const DIFF_PAGE: u16 = 10;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pane {
    Files,
    Diff,
}

/// Identifies a row: the same path can be listed once staged and once unstaged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntryKey {
    path: String,
    staged: bool,
}

impl EntryKey {
    fn of(change: &FileChange) -> Self {
        Self {
            path: change.path.clone(),
            staged: change.staged,
        }
    }
}

struct LoadedDiff {
    key: EntryKey,
    result: Result<FileDiff, String>,
}

/// A hunk chosen for staging, unstaging or discarding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HunkChoice {
    index: usize,
    total: usize,
    patch: Vec<u8>,
    header: String,
    lines: Vec<DiffLine>,
    added: usize,
    removed: usize,
}

/// The pending discard shown in the confirmation dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Discard {
    change: FileChange,
    hunk: Option<HunkChoice>,
}

/// A working-tree change run on a background thread.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Operation {
    StageFile(FileChange),
    UnstageFile {
        change: FileChange,
        unborn: bool,
    },
    StageHunk {
        change: FileChange,
        hunk: HunkChoice,
    },
    UnstageHunk {
        change: FileChange,
        hunk: HunkChoice,
    },
    Discard(Discard),
}

impl Operation {
    fn label(&self) -> &'static str {
        match self {
            Self::StageFile(_) | Self::StageHunk { .. } => "staging…",
            Self::UnstageFile { .. } | Self::UnstageHunk { .. } => "unstaging…",
            Self::Discard(_) => "discarding…",
        }
    }

    /// Runs the operation and describes the outcome for the notice line.
    fn run(self, repo: &std::path::Path) -> Result<String, String> {
        let next = "Press r to refresh, then try again.";
        match self {
            Self::StageFile(change) => stage_file(repo, &change)
                .map(|()| format!("Staged {}.", change.path))
                .map_err(|cause| format!("Could not stage {}. {cause} {next}", change.path)),
            Self::UnstageFile { change, unborn } => unstage_file(repo, &change, unborn)
                .map(|()| format!("Unstaged {}.", change.path))
                .map_err(|cause| format!("Could not unstage {}. {cause} {next}", change.path)),
            Self::StageHunk { change, hunk } => apply_to_index(repo, &hunk.patch, false)
                .map(|()| {
                    format!(
                        "Staged hunk {} of {} in {}.",
                        hunk.index + 1,
                        hunk.total,
                        change.path
                    )
                })
                .map_err(|cause| {
                    format!(
                        "Could not stage this hunk of {}. {cause} {next}",
                        change.path
                    )
                }),
            Self::UnstageHunk { change, hunk } => apply_to_index(repo, &hunk.patch, true)
                .map(|()| {
                    format!(
                        "Unstaged hunk {} of {} in {}.",
                        hunk.index + 1,
                        hunk.total,
                        change.path
                    )
                })
                .map_err(|cause| {
                    format!(
                        "Could not unstage this hunk of {}. {cause} {next}",
                        change.path
                    )
                }),
            Self::Discard(Discard { change, hunk }) => {
                let path = change.path.clone();
                let result = match &hunk {
                    Some(hunk) => discard_hunk(repo, &change, &hunk.patch),
                    None => discard_file(repo, &change),
                };
                result
                    .map(|snapshot| {
                        format!("Discarded. Undo with: git checkout {snapshot} -- {path}")
                    })
                    .map_err(|cause| format!("Nothing was discarded in {path}. {cause} {next}"))
            }
        }
    }
}

/// Background work requested by the Changes screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChangesRequest {
    Load,
    Diff(FileChange),
    Run(Operation),
}

pub enum ChangesEvent {
    Loaded(Result<WorkingTree, String>),
    Diff(EntryKey, Result<FileDiff, String>),
    Done(Result<String, String>),
}

/// Runs `request` on a worker thread. Results carry the repository path so a
/// screen opened later for another project can ignore them.
pub fn spawn_request(
    repo: PathBuf,
    request: ChangesRequest,
    sender: mpsc::Sender<(PathBuf, ChangesEvent)>,
) {
    thread::spawn(move || {
        let event = match request {
            ChangesRequest::Load => ChangesEvent::Loaded(read_working_tree(&repo)),
            ChangesRequest::Diff(change) => {
                ChangesEvent::Diff(EntryKey::of(&change), diff_for(&repo, &change))
            }
            ChangesRequest::Run(operation) => ChangesEvent::Done(operation.run(&repo)),
        };
        let _ = sender.send((repo, event));
    });
}

pub struct ChangesView {
    pub path: PathBuf,
    tree: Option<WorkingTree>,
    load_error: Option<String>,
    loading: bool,
    selected: usize,
    focus: Pane,
    diff: Option<LoadedDiff>,
    hunk: usize,
    diff_scroll: u16,
    busy: Option<&'static str>,
    confirm: Option<Discard>,
    notice: Option<Notice>,
    /// Background work for the run loop to start; only one is pending at a time.
    pub request: Option<ChangesRequest>,
    /// Esc was pressed: return to the project list.
    pub closed: bool,
    /// `c` was pressed: the app opens the Commit screen.
    pub open_commit: bool,
    /// `l` was pressed: the app opens the Commits screen.
    pub open_commits: bool,
    /// Subject and body of a cancelled commit message, restored on the next `c`.
    pub draft: Option<(String, String)>,
    pub should_quit: bool,
}

impl ChangesView {
    /// A view for `path` that asks the run loop to read the working tree.
    pub fn open(path: PathBuf) -> Self {
        Self {
            path,
            tree: None,
            load_error: None,
            loading: true,
            selected: 0,
            focus: Pane::Files,
            diff: None,
            hunk: 0,
            diff_scroll: 0,
            busy: None,
            confirm: None,
            notice: None,
            request: Some(ChangesRequest::Load),
            closed: false,
            open_commit: false,
            open_commits: false,
            draft: None,
            should_quit: false,
        }
    }

    /// Staged files and branch tracking, for the Commit screen.
    pub fn commit_context(&self) -> (Vec<FileChange>, Option<BranchTracking>) {
        self.tree.as_ref().map_or((Vec::new(), None), |tree| {
            (tree.staged.clone(), Some(tree.tracking.clone()))
        })
    }

    /// Branch tracking for the title bar of screens opened from here.
    pub fn tracking(&self) -> Option<BranchTracking> {
        self.tree.as_ref().map(|tree| tree.tracking.clone())
    }

    /// Back from the Commit screen after a commit: show the outcome and reload.
    pub fn commit_finished(&mut self, notice: Notice) {
        self.draft = None;
        self.notice = Some(notice);
        self.reload();
    }

    fn entries(&self) -> Vec<&FileChange> {
        self.tree
            .iter()
            .flat_map(|tree| tree.staged.iter().chain(&tree.unstaged))
            .collect()
    }

    fn selected_change(&self) -> Option<&FileChange> {
        self.entries().get(self.selected).copied()
    }

    /// The loaded diff of the selected entry, if it has arrived.
    fn selected_diff(&self) -> Option<&FileDiff> {
        let key = EntryKey::of(self.selected_change()?);
        self.diff
            .as_ref()
            .filter(|loaded| loaded.key == key)
            .and_then(|loaded| loaded.result.as_ref().ok())
    }

    pub fn apply(&mut self, event: ChangesEvent) {
        match event {
            ChangesEvent::Loaded(Ok(tree)) => {
                let previous = self.selected_change().map(|change| {
                    let staged_count = self.tree.as_ref().map_or(0, |tree| tree.staged.len());
                    let section_index = if change.staged {
                        self.selected
                    } else {
                        self.selected - staged_count
                    };
                    (EntryKey::of(change), section_index)
                });
                self.tree = Some(tree);
                self.loading = false;
                self.load_error = None;
                self.reselect(previous);
            }
            ChangesEvent::Loaded(Err(error)) => {
                self.loading = false;
                self.load_error = Some(error);
            }
            ChangesEvent::Diff(key, result) => {
                if self
                    .selected_change()
                    .is_some_and(|change| EntryKey::of(change) == key)
                {
                    self.diff = Some(LoadedDiff { key, result });
                    let hunks = self.selected_diff().map_or(0, |diff| diff.hunks.len());
                    self.hunk = self.hunk.min(hunks.saturating_sub(1));
                }
            }
            ChangesEvent::Done(result) => {
                self.busy = None;
                self.notice = Some(match result {
                    Ok(text) => Notice::info(text),
                    Err(text) => Notice::error(text),
                });
                self.reload();
            }
        }
    }

    /// Keeps the cursor on the same entry after a reload; when that entry moved to
    /// the other list, stays at the same position in its old list so repeated
    /// stage presses walk down the files.
    fn reselect(&mut self, previous: Option<(EntryKey, usize)>) {
        let Some(tree) = &self.tree else { return };
        let staged = tree.staged.len();
        let total = staged + tree.unstaged.len();
        let same_key = previous.as_ref().and_then(|(key, _)| {
            self.entries()
                .iter()
                .position(|change| EntryKey::of(change) == *key)
        });
        let selected = match (same_key, previous) {
            (Some(index), _) => index,
            (None, Some((key, position))) => {
                let (start, length) = if key.staged {
                    (0, staged)
                } else {
                    (staged, total - staged)
                };
                if length > 0 {
                    start + position.min(length - 1)
                } else {
                    position.min(total.saturating_sub(1))
                }
            }
            (None, None) => 0,
        };
        let kept = same_key.is_some();
        self.selected = selected;
        if !kept {
            self.hunk = 0;
            self.diff_scroll = 0;
            self.diff = None;
        }
        self.request_diff();
    }

    fn request_diff(&mut self) {
        self.request = self.selected_change().cloned().map(ChangesRequest::Diff);
    }

    fn reload(&mut self) {
        self.loading = true;
        self.request = Some(ChangesRequest::Load);
    }

    fn select(&mut self, index: usize) {
        if index == self.selected {
            return;
        }
        self.selected = index;
        self.hunk = 0;
        self.diff_scroll = 0;
        self.diff = None;
        self.request_diff();
    }

    fn move_cursor(&mut self, delta: isize) {
        match self.focus {
            Pane::Files => {
                let count = self.entries().len();
                if count > 0 {
                    let next = (self.selected as isize + delta).clamp(0, count as isize - 1);
                    self.select(next as usize);
                }
            }
            Pane::Diff => {
                let hunks = self.selected_diff().map_or(0, |diff| diff.hunks.len());
                if hunks > 0 {
                    let next = (self.hunk as isize + delta).clamp(0, hunks as isize - 1) as usize;
                    if next != self.hunk {
                        self.hunk = next;
                        self.diff_scroll = 0;
                    }
                }
            }
        }
    }

    fn current_hunk(&self) -> Option<HunkChoice> {
        let diff = self.selected_diff()?;
        let hunk = diff.hunks.get(self.hunk)?;
        Some(HunkChoice {
            index: self.hunk,
            total: diff.hunks.len(),
            patch: diff.hunk_patch(self.hunk)?,
            header: hunk.header.clone(),
            lines: hunk.lines.clone(),
            added: hunk.added(),
            removed: hunk.removed(),
        })
    }

    fn run(&mut self, operation: Operation) {
        self.busy = Some(operation.label());
        self.notice = None;
        self.request = Some(ChangesRequest::Run(operation));
    }

    fn say(&mut self, text: &str) {
        self.notice = Some(Notice::info(text));
    }

    fn stage_hunk(&mut self) {
        let Some(change) = self.selected_change().cloned() else {
            return;
        };
        if change.staged {
            return self.say("Already staged. Press u to unstage it.");
        }
        if !change.supports_hunks() || self.selected_diff().is_some_and(|diff| diff.binary) {
            return self.run(Operation::StageFile(change));
        }
        match self.current_hunk() {
            Some(hunk) => self.run(Operation::StageHunk { change, hunk }),
            None => self.say("The diff is still loading. Try again in a moment."),
        }
    }

    fn stage_file(&mut self) {
        let Some(change) = self.selected_change().cloned() else {
            return;
        };
        if change.staged {
            return self.say("Already staged. Press u to unstage it.");
        }
        self.run(Operation::StageFile(change));
    }

    fn unstage(&mut self) {
        let Some(change) = self.selected_change().cloned() else {
            return;
        };
        if !change.staged {
            return self.say("Not staged yet. Press s to stage a hunk or a for the file.");
        }
        let hunk = (self.focus == Pane::Diff && change.supports_hunks())
            .then(|| self.current_hunk())
            .flatten();
        match hunk {
            Some(hunk) => self.run(Operation::UnstageHunk { change, hunk }),
            None => {
                let unborn = self.tree.as_ref().is_some_and(|tree| tree.tracking.unborn);
                self.run(Operation::UnstageFile { change, unborn });
            }
        }
    }

    fn discard(&mut self) {
        let Some(change) = self.selected_change().cloned() else {
            return;
        };
        if change.staged {
            return self.say("Discard only touches unstaged changes. Press u to unstage first.");
        }
        if change.state == FileState::Conflicted {
            return self
                .say("Resolve the conflict in your editor, then press a to mark it resolved.");
        }
        let hunk = (self.focus == Pane::Diff && change.supports_hunks())
            .then(|| self.current_hunk())
            .flatten();
        self.confirm = Some(Discard { change, hunk });
    }

    pub fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        let control = modifiers.contains(KeyModifiers::CONTROL);
        if control && code == KeyCode::Char('c') {
            self.should_quit = true;
            return;
        }

        if let Some(discard) = &self.confirm {
            match code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    let discard = discard.clone();
                    self.confirm = None;
                    self.run(Operation::Discard(discard));
                }
                KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => self.confirm = None,
                _ => {}
            }
            return;
        }

        let mutates = !control && matches!(code, KeyCode::Char('s' | 'a' | 'u' | 'x' | 'r' | 'c'));
        if mutates && self.busy.is_some() {
            return self.say("Still working on the last change…");
        }

        match code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Esc => self.closed = true,
            KeyCode::Tab | KeyCode::BackTab => {
                self.focus = match self.focus {
                    Pane::Files => Pane::Diff,
                    Pane::Diff => Pane::Files,
                }
            }
            KeyCode::Enter if self.focus == Pane::Files => self.focus = Pane::Diff,
            KeyCode::Up | KeyCode::Char('k') => self.move_cursor(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_cursor(1),
            KeyCode::PageDown => self.diff_scroll = self.diff_scroll.saturating_add(DIFF_PAGE),
            KeyCode::Char('d') if control => {
                self.diff_scroll = self.diff_scroll.saturating_add(DIFF_PAGE)
            }
            KeyCode::PageUp => self.diff_scroll = self.diff_scroll.saturating_sub(DIFF_PAGE),
            KeyCode::Char('u') if control => {
                self.diff_scroll = self.diff_scroll.saturating_sub(DIFF_PAGE)
            }
            KeyCode::Char('r') => {
                self.notice = None;
                self.reload();
            }
            KeyCode::Char('s') => self.stage_hunk(),
            KeyCode::Char('a') => self.stage_file(),
            KeyCode::Char('u') => self.unstage(),
            KeyCode::Char('x') => self.discard(),
            KeyCode::Char('l') => self.open_commits = true,
            KeyCode::Char('c') if self.tree.is_some() => {
                self.notice = None;
                self.open_commit = true;
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests;
