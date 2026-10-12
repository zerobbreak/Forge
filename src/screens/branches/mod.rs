//! 04 · Branches & worktrees — list, switch, create, rename and delete branches.

mod view;

pub use view::draw_branches;

use crate::{
    editor::TextArea,
    git::{
        branches::{
            Branch, BranchKind, BranchList, SwitchTarget, check_branch_name, create_branch,
            delete_branch, read_branches, rename_branch, switch_branch,
        },
        repository::BranchTracking,
    },
    ui::Notice,
};
use crossterm::event::{KeyCode, KeyModifiers};
use std::{path::PathBuf, sync::mpsc, thread};

/// A change to the repository, run on a background thread.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Operation {
    Switch { target: SwitchTarget, stash: bool },
    Create { name: String, start: String },
    Rename { old: String, new: String },
    Delete { name: String, force: bool },
}

impl Operation {
    fn label(&self) -> &'static str {
        match self {
            Self::Switch { .. } => "switching…",
            Self::Create { .. } => "creating…",
            Self::Rename { .. } => "renaming…",
            Self::Delete { .. } => "deleting…",
        }
    }

    /// The branch the cursor should rest on once the operation is done.
    fn select_after(&self) -> Option<String> {
        match self {
            Self::Switch { target, .. } => Some(target.name.clone()),
            Self::Create { name, .. } => Some(name.clone()),
            Self::Rename { new, .. } => Some(new.clone()),
            Self::Delete { .. } => None,
        }
    }

    fn run(self, repo: &std::path::Path) -> Result<String, String> {
        let next = "Press r to refresh, then try again.";
        match self {
            Self::Switch { target, stash } => switch_branch(repo, &target, stash)
                .map_err(|cause| format!("Could not switch to {}. {cause} {next}", target.name)),
            Self::Create { name, start } => create_branch(repo, &name, &start)
                .map_err(|cause| format!("Could not create {name}. {cause} {next}")),
            Self::Rename { old, new } => rename_branch(repo, &old, &new)
                .map_err(|cause| format!("Could not rename {old}. {cause} {next}")),
            Self::Delete { name, force } => delete_branch(repo, &name, force)
                .map_err(|cause| format!("Nothing was deleted. {cause} {next}")),
        }
    }
}

/// Background work requested by the Branches screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BranchesRequest {
    Load,
    Run(Operation),
}

pub enum BranchesEvent {
    Loaded(Result<BranchList, String>),
    Done(Result<String, String>),
}

/// Runs `request` on a worker thread. Results carry the repository path so a
/// screen opened later for another project can ignore them.
pub fn spawn_request(
    repo: PathBuf,
    request: BranchesRequest,
    sender: mpsc::Sender<(PathBuf, BranchesEvent)>,
) {
    thread::spawn(move || {
        let event = match request {
            BranchesRequest::Load => BranchesEvent::Loaded(read_branches(&repo)),
            BranchesRequest::Run(operation) => BranchesEvent::Done(operation.run(&repo)),
        };
        let _ = sender.send((repo, event));
    });
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum NamePurpose {
    /// A new branch starting at `start` (a full ref), shown to the user as `from`.
    Create {
        start: String,
        from: String,
    },
    Rename {
        old: String,
    },
}

struct NameBox {
    purpose: NamePurpose,
    text: TextArea,
    error: Option<String>,
}

/// The pending delete shown in the confirmation dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
struct DeleteChoice {
    name: String,
    short: String,
    merged: bool,
    unique: Option<usize>,
    /// The branch the deletion is judged against.
    into: String,
}

/// A switch that would start with uncommitted changes.
#[derive(Clone, Debug, PartialEq, Eq)]
struct DirtyChoice {
    target: SwitchTarget,
    files: usize,
}

enum Mode {
    Browse,
    Name(NameBox),
    Delete(DeleteChoice),
    Dirty(DirtyChoice),
}

pub struct BranchesView {
    pub path: PathBuf,
    tracking: Option<BranchTracking>,
    list: Option<BranchList>,
    load_error: Option<String>,
    loading: bool,
    /// Index into the list's branches.
    selected: usize,
    /// The branch to put the cursor on after the next load.
    select_after: Option<String>,
    mode: Mode,
    busy: Option<&'static str>,
    notice: Option<Notice>,
    /// On narrow terminals the detail replaces the list while this is set.
    detail_open: bool,
    /// Background work for the run loop to start.
    pub request: Option<BranchesRequest>,
    /// Esc was pressed: return to Changes.
    pub closed: bool,
    pub should_quit: bool,
}

impl BranchesView {
    /// A view for `path` that asks the run loop to read the branches.
    pub fn open(path: PathBuf, tracking: Option<BranchTracking>) -> Self {
        Self {
            path,
            tracking,
            list: None,
            load_error: None,
            loading: true,
            selected: 0,
            select_after: None,
            mode: Mode::Browse,
            busy: None,
            notice: None,
            detail_open: false,
            request: Some(BranchesRequest::Load),
            closed: false,
            should_quit: false,
        }
    }

    fn branches(&self) -> &[Branch] {
        self.list.as_ref().map_or(&[], |list| &list.branches)
    }

    fn selected_branch(&self) -> Option<&Branch> {
        self.branches().get(self.selected)
    }

    fn current_name(&self) -> String {
        self.list
            .as_ref()
            .and_then(|list| list.current())
            .map_or_else(|| "HEAD".to_owned(), |branch| branch.name.clone())
    }

    fn reload(&mut self) {
        if self.select_after.is_none() {
            self.select_after = self.selected_branch().map(|branch| branch.name.clone());
        }
        self.loading = true;
        self.request = Some(BranchesRequest::Load);
    }

    pub fn apply(&mut self, event: BranchesEvent) {
        match event {
            BranchesEvent::Loaded(Ok(list)) => {
                self.loading = false;
                self.load_error = None;
                self.tracking = Some(match list.current() {
                    Some(branch) => BranchTracking {
                        branch: branch.name.clone(),
                        upstream: branch.upstream.is_some() && !branch.gone,
                        ahead: branch.ahead,
                        behind: branch.behind,
                        unborn: false,
                    },
                    None => BranchTracking::default(),
                });
                let wanted = self.select_after.take();
                self.selected = wanted
                    .and_then(|name| list.branches.iter().position(|branch| branch.name == name))
                    .unwrap_or_else(|| self.selected.min(list.branches.len().saturating_sub(1)));
                self.list = Some(list);
            }
            BranchesEvent::Loaded(Err(error)) => {
                self.loading = false;
                self.load_error = Some(error);
            }
            BranchesEvent::Done(result) => {
                self.busy = None;
                self.notice = Some(match result {
                    Ok(text) => Notice::info(text),
                    Err(text) => {
                        // Nothing moved, so keep the cursor where it was.
                        self.select_after = None;
                        Notice::error(text)
                    }
                });
                self.reload();
            }
        }
    }

    fn say(&mut self, text: impl Into<String>) {
        self.notice = Some(Notice::info(text));
    }

    fn run(&mut self, operation: Operation) {
        self.busy = Some(operation.label());
        self.notice = None;
        self.mode = Mode::Browse;
        self.select_after = operation.select_after();
        self.request = Some(BranchesRequest::Run(operation));
    }

    fn move_cursor(&mut self, delta: isize) {
        let count = self.branches().len();
        if count > 0 {
            self.selected = (self.selected as isize + delta).clamp(0, count as isize - 1) as usize;
        }
    }

    fn switch_selected(&mut self) {
        let Some(branch) = self.selected_branch().cloned() else {
            return;
        };
        if branch.current {
            return self.say(format!("Already on {}.", branch.name));
        }
        if let Some(folder) = &branch.worktree {
            return self.say(format!(
                "{} is checked out in another worktree ({folder}). Open that folder to work on it.",
                branch.name
            ));
        }
        let target = match branch.kind {
            BranchKind::Local => SwitchTarget {
                name: branch.name.clone(),
                track: None,
            },
            BranchKind::Remote => {
                let local = branch
                    .name
                    .split_once('/')
                    .map_or(branch.name.as_str(), |(_, name)| name)
                    .to_owned();
                let exists = self
                    .list
                    .iter()
                    .flat_map(BranchList::locals)
                    .any(|candidate| candidate.name == local);
                SwitchTarget {
                    track: (!exists).then(|| branch.name.clone()),
                    name: local,
                }
            }
        };
        let files = self.list.as_ref().map_or(0, |list| list.dirty_files);
        if files > 0 {
            self.mode = Mode::Dirty(DirtyChoice { target, files });
        } else {
            self.run(Operation::Switch {
                target,
                stash: false,
            });
        }
    }

    fn start_create(&mut self) {
        let Some(branch) = self.selected_branch() else {
            return self
                .say("There is no commit to branch from yet. Make a commit in Changes first.");
        };
        let start = match branch.kind {
            BranchKind::Local => format!("refs/heads/{}", branch.name),
            BranchKind::Remote => format!("refs/remotes/{}", branch.name),
        };
        let from = branch.name.clone();
        self.mode = Mode::Name(NameBox {
            purpose: NamePurpose::Create { start, from },
            text: TextArea::single_line(""),
            error: None,
        });
    }

    fn start_rename(&mut self) {
        let Some(branch) = self.selected_branch() else {
            return;
        };
        if branch.kind == BranchKind::Remote {
            return self.say("Remote branches are renamed on the remote, not here.");
        }
        let old = branch.name.clone();
        self.mode = Mode::Name(NameBox {
            text: TextArea::single_line(&old),
            purpose: NamePurpose::Rename { old },
            error: None,
        });
    }

    fn start_delete(&mut self) {
        let Some(branch) = self.selected_branch().cloned() else {
            return;
        };
        if branch.kind == BranchKind::Remote {
            let (remote, name) = branch
                .name
                .split_once('/')
                .unwrap_or(("origin", &branch.name));
            return self.say(format!(
                "Deleting remote branches is not supported here. In a shell: git push {remote} --delete {name}"
            ));
        }
        if branch.current {
            return self
                .say("You are on this branch. Switch to another one first, then delete it.");
        }
        if let Some(folder) = &branch.worktree {
            return self.say(format!(
                "{} is checked out in another worktree ({folder}) and cannot be deleted.",
                branch.name
            ));
        }
        self.mode = Mode::Delete(DeleteChoice {
            short: branch.short.clone(),
            merged: branch.merged,
            unique: branch.unique,
            into: self.current_name(),
            name: branch.name,
        });
    }

    fn submit_name(&mut self) {
        let Mode::Name(input) = &mut self.mode else {
            return;
        };
        let name = input.text.text().trim().to_owned();
        let taken = self
            .list
            .iter()
            .flat_map(BranchList::locals)
            .any(|branch| branch.name == name);
        let checked = check_branch_name(&name).and_then(|()| match &input.purpose {
            NamePurpose::Rename { old } if *old == name => {
                Err("That is already its name.".to_owned())
            }
            _ if taken => Err(format!("A branch named {name} already exists.")),
            _ => Ok(()),
        });
        if let Err(error) = checked {
            input.error = Some(error);
            return;
        }
        let operation = match &input.purpose {
            NamePurpose::Create { start, .. } => Operation::Create {
                name,
                start: start.clone(),
            },
            NamePurpose::Rename { old } => Operation::Rename {
                old: old.clone(),
                new: name,
            },
        };
        self.run(operation);
    }

    pub fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        let control = modifiers.contains(KeyModifiers::CONTROL);
        if control && code == KeyCode::Char('c') {
            self.should_quit = true;
            return;
        }

        match &mut self.mode {
            Mode::Name(input) => {
                match code {
                    KeyCode::Esc => self.mode = Mode::Browse,
                    KeyCode::Enter => self.submit_name(),
                    _ => {
                        if input.text.handle_key(code, modifiers) {
                            input.error = None;
                        }
                    }
                }
                return;
            }
            Mode::Delete(choice) => {
                match code {
                    KeyCode::Char('y' | 'Y') => {
                        let operation = Operation::Delete {
                            name: choice.name.clone(),
                            force: !choice.merged,
                        };
                        self.run(operation);
                    }
                    KeyCode::Esc | KeyCode::Char('n' | 'N') => self.mode = Mode::Browse,
                    _ => {}
                }
                return;
            }
            Mode::Dirty(choice) => {
                let target = choice.target.clone();
                match code {
                    KeyCode::Char('s' | 'S') => self.run(Operation::Switch {
                        target,
                        stash: true,
                    }),
                    KeyCode::Char('c' | 'C') => self.run(Operation::Switch {
                        target,
                        stash: false,
                    }),
                    KeyCode::Esc => self.mode = Mode::Browse,
                    _ => {}
                }
                return;
            }
            Mode::Browse => {}
        }

        let mutates = !control && matches!(code, KeyCode::Enter | KeyCode::Char('n' | 'R' | 'x'));
        if mutates && self.busy.is_some() {
            return self.say("Still working on the last change…");
        }

        match code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Esc if self.detail_open => self.detail_open = false,
            KeyCode::Esc => self.closed = true,
            KeyCode::Tab | KeyCode::BackTab => self.detail_open = !self.detail_open,
            KeyCode::Up | KeyCode::Char('k') => self.move_cursor(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_cursor(1),
            KeyCode::PageUp => self.move_cursor(-10),
            KeyCode::PageDown => self.move_cursor(10),
            KeyCode::Home | KeyCode::Char('g') => self.selected = 0,
            KeyCode::End | KeyCode::Char('G') => {
                self.selected = self.branches().len().saturating_sub(1);
            }
            KeyCode::Enter => self.switch_selected(),
            KeyCode::Char('n') => self.start_create(),
            KeyCode::Char('R') => self.start_rename(),
            KeyCode::Char('x') => self.start_delete(),
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
