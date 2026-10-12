//! 02 · Commit — compose the message, run hooks, optionally push.

mod view;

pub use view::draw_commit;

use crate::{
    editor::TextArea,
    git::{
        changes::FileChange,
        commit::{CommitInfo, CommitOptions, commit, push, read_commit_info},
        repository::BranchTracking,
    },
    ui::Notice,
};
use crossterm::event::{KeyCode, KeyModifiers};
use std::{path::PathBuf, sync::mpsc, thread};

/// Conventional-commit types offered as chips, as in the design.
pub const TYPES: [&str; 6] = ["feat", "fix", "refactor", "perf", "docs", "chore"];
/// Subjects longer than this are flagged; git itself accepts any length.
pub const SUBJECT_LIMIT: usize = 72;
/// Output lines kept from hooks and push.
const OUTPUT_LINES: usize = 50;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Type,
    Subject,
    Body,
    Options,
}

impl Field {
    const ORDER: [Self; 4] = [Self::Type, Self::Subject, Self::Body, Self::Options];

    fn step(self, delta: isize) -> Self {
        let index = Self::ORDER
            .iter()
            .position(|field| *field == self)
            .unwrap_or(0) as isize;
        Self::ORDER[(index + delta).rem_euclid(Self::ORDER.len() as isize) as usize]
    }
}

/// The toggles under the message, in display order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Toggle {
    Amend,
    Sign,
    PushAfter,
    SkipHooks,
}

impl Toggle {
    pub const ORDER: [Self; 4] = [Self::Amend, Self::Sign, Self::PushAfter, Self::SkipHooks];

    pub fn label(self) -> &'static str {
        match self {
            Self::Amend => "amend",
            Self::Sign => "sign",
            Self::PushAfter => "push",
            Self::SkipHooks => "skip hooks",
        }
    }

    /// The letter that flips this toggle while the options row is focused.
    fn key(self) -> char {
        match self {
            Self::Amend => 'e',
            Self::Sign => 'g',
            Self::PushAfter => 'p',
            Self::SkipHooks => 'n',
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Editing,
    Committing,
    Pushing,
}

/// What happened to the last `git commit` attempt, for the Hooks section.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HookRun {
    NotRun,
    Passed,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommitRequest {
    Info,
    Commit {
        message: String,
        options: CommitOptions,
    },
    Push,
}

pub enum CommitEvent {
    Info(Result<CommitInfo, String>),
    Line(String),
    Committed(Result<String, String>),
    Pushed(Result<String, String>),
}

/// How the screen was left; the Changes screen keeps a cancelled draft.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommitExit {
    Cancelled { subject: String, body: String },
    Done(Notice),
}

pub fn spawn_request(
    repo: PathBuf,
    request: CommitRequest,
    sender: mpsc::Sender<(PathBuf, CommitEvent)>,
) {
    thread::spawn(move || {
        let lines = sender.clone();
        let line_repo = repo.clone();
        let mut on_line = move |line: String| {
            let _ = lines.send((line_repo.clone(), CommitEvent::Line(line)));
        };
        let event = match request {
            CommitRequest::Info => CommitEvent::Info(read_commit_info(&repo)),
            CommitRequest::Commit { message, options } => {
                CommitEvent::Committed(commit(&repo, &message, options, &mut on_line))
            }
            CommitRequest::Push => CommitEvent::Pushed(push(&repo, &mut on_line)),
        };
        let _ = sender.send((repo, event));
    });
}

/// The conventional type at the start of `subject` (`feat(scope)!: …`), if it is one of [`TYPES`].
pub fn subject_type(subject: &str) -> Option<&'static str> {
    let (prefix, _) = split_type(subject)?;
    TYPES.iter().copied().find(|kind| *kind == prefix)
}

/// Splits `type(scope)!: description` into the type word and everything after it.
fn split_type(subject: &str) -> Option<(&str, &str)> {
    let word_end = subject
        .find(|character: char| !character.is_ascii_alphanumeric())
        .filter(|end| *end > 0)?;
    let (word, rest) = subject.split_at(word_end);
    let mut after = rest;
    if let Some(scoped) = after.strip_prefix('(') {
        let close = scoped.find(')')?;
        after = &scoped[close + 1..];
    }
    let after = after.strip_prefix('!').unwrap_or(after);
    after.starts_with(": ").then_some((word, rest))
}

/// Rewrites the subject's type prefix: replaces the type word (keeping the scope),
/// adds `type: ` when there is none, or removes the whole prefix for `None`.
pub fn with_type(subject: &str, kind: Option<&str>) -> String {
    let current = subject_type(subject).and_then(|_| split_type(subject));
    match (current, kind) {
        (Some((_, rest)), Some(kind)) => format!("{kind}{rest}"),
        (Some((_, rest)), None) => rest
            .split_once(": ")
            .map_or_else(String::new, |(_, description)| description.to_owned()),
        (None, Some(kind)) => format!("{kind}: {subject}"),
        (None, None) => subject.to_owned(),
    }
}

/// `Token: value` lines in the body's last paragraph, as git treats trailers.
pub fn trailers(body: &str) -> Vec<(String, String)> {
    let last = body
        .trim_end()
        .rsplit("\n\n")
        .next()
        .unwrap_or_default()
        .trim();
    let parsed: Vec<_> = last
        .lines()
        .map(|line| {
            line.split_once(": ").filter(|(token, value)| {
                !token.is_empty()
                    && !value.trim().is_empty()
                    && token
                        .chars()
                        .all(|character| character.is_ascii_alphanumeric() || character == '-')
            })
        })
        .collect();
    if parsed.is_empty() || parsed.iter().any(Option::is_none) {
        return Vec::new();
    }
    parsed
        .into_iter()
        .flatten()
        .map(|(token, value)| (token.to_owned(), value.trim().to_owned()))
        .collect()
}

pub struct CommitView {
    pub path: PathBuf,
    staged: Vec<FileChange>,
    tracking: Option<BranchTracking>,
    info: Option<Result<CommitInfo, String>>,
    focus: Field,
    subject: TextArea,
    body: TextArea,
    option: usize,
    amend: bool,
    /// `None` follows `commit.gpgsign`.
    sign: Option<bool>,
    push_after: bool,
    skip_hooks: bool,
    phase: Phase,
    hooks: HookRun,
    output: Vec<String>,
    summary: Option<String>,
    notice: Option<Notice>,
    pub request: Option<CommitRequest>,
    pub exit: Option<CommitExit>,
    pub should_quit: bool,
}

impl CommitView {
    /// Opens the screen for `path`, restoring a cancelled draft if there is one.
    pub fn open(
        path: PathBuf,
        staged: Vec<FileChange>,
        tracking: Option<BranchTracking>,
        draft: Option<(String, String)>,
    ) -> Self {
        let (subject, body) = draft.unwrap_or_default();
        Self {
            path,
            staged,
            tracking,
            info: None,
            focus: Field::Subject,
            subject: TextArea::single_line(&subject),
            body: TextArea::multi_line(&body),
            option: 0,
            amend: false,
            sign: None,
            push_after: false,
            skip_hooks: false,
            phase: Phase::Editing,
            hooks: HookRun::NotRun,
            output: Vec::new(),
            summary: None,
            notice: None,
            request: Some(CommitRequest::Info),
            exit: None,
            should_quit: false,
        }
    }

    fn loaded_info(&self) -> Option<&CommitInfo> {
        self.info.as_ref().and_then(|info| info.as_ref().ok())
    }

    pub fn signing(&self) -> bool {
        self.sign
            .unwrap_or_else(|| self.loaded_info().is_some_and(|info| info.sign_by_default))
    }

    pub fn toggled(&self, toggle: Toggle) -> bool {
        match toggle {
            Toggle::Amend => self.amend,
            Toggle::Sign => self.signing(),
            Toggle::PushAfter => self.push_after,
            Toggle::SkipHooks => self.skip_hooks,
        }
    }

    pub fn apply(&mut self, event: CommitEvent) {
        match event {
            CommitEvent::Info(info) => self.info = Some(info),
            CommitEvent::Line(line) => {
                self.output.push(line);
                if self.output.len() > OUTPUT_LINES {
                    self.output.remove(0);
                }
            }
            CommitEvent::Committed(Ok(summary)) => {
                self.hooks = HookRun::Passed;
                if self.push_after {
                    self.phase = Phase::Pushing;
                    self.summary = Some(summary);
                    self.request = Some(CommitRequest::Push);
                } else {
                    self.exit = Some(CommitExit::Done(Notice::info(format!(
                        "Committed {summary}."
                    ))));
                }
            }
            CommitEvent::Committed(Err(error)) => {
                self.phase = Phase::Editing;
                self.hooks = HookRun::Failed;
                self.notice = Some(Notice::error(format!(
                    "Commit failed: {error} Fix it (see output on the left), then press ctrl+s; your message is kept."
                )));
            }
            CommitEvent::Pushed(result) => {
                let summary = self.summary.take().unwrap_or_default();
                self.exit = Some(CommitExit::Done(match result {
                    Ok(pushed) => Notice::info(format!("Committed {summary}. {pushed}")),
                    Err(error) => Notice::error(format!(
                        "Committed {summary}, but the push failed: {error} Check your network or credentials, then push from a shell."
                    )),
                }));
            }
        }
    }

    pub fn message(&self) -> String {
        let subject = self.subject.text().trim().to_owned();
        let body = self.body.text().trim_end().to_owned();
        if body.trim().is_empty() {
            subject
        } else {
            format!("{subject}\n\n{body}\n")
        }
    }

    fn submit(&mut self) {
        if self.phase != Phase::Editing {
            return;
        }
        if self.subject.text().trim().is_empty() {
            self.focus = Field::Subject;
            self.notice = Some(Notice::error(
                "There is no subject. Write a one-line summary in Subject, then press ctrl+s.",
            ));
            return;
        }
        if self.staged.is_empty() && !self.amend {
            self.notice = Some(Notice::error(
                "Nothing is staged. Press esc, stage changes with s or a, then press c again.",
            ));
            return;
        }
        if self.push_after && self.loaded_info().is_some_and(|info| !info.has_remote) {
            self.notice = Some(Notice::error(
                "This repository has no remote, so it cannot push. Turn off \"push after\" (p) or add a remote.",
            ));
            return;
        }
        let sign = self
            .sign
            .filter(|sign| Some(*sign) != self.loaded_info().map(|info| info.sign_by_default));
        self.phase = Phase::Committing;
        self.output.clear();
        self.notice = None;
        self.request = Some(CommitRequest::Commit {
            message: self.message(),
            options: CommitOptions {
                amend: self.amend,
                sign,
                skip_hooks: self.skip_hooks,
            },
        });
    }

    fn cycle_type(&mut self, delta: isize) {
        let current = subject_type(&self.subject.text());
        let choices: Vec<Option<&str>> = std::iter::once(None)
            .chain(TYPES.iter().copied().map(Some))
            .collect();
        let index = choices
            .iter()
            .position(|choice| *choice == current)
            .unwrap_or(0) as isize;
        let next = choices[(index + delta).rem_euclid(choices.len() as isize) as usize];
        let subject = with_type(&self.subject.text(), next);
        self.subject.set_text(&subject);
    }

    fn flip(&mut self, toggle: Toggle) {
        match toggle {
            Toggle::Amend => {
                self.amend = !self.amend;
                self.on_amend_changed();
            }
            Toggle::Sign => self.sign = Some(!self.signing()),
            Toggle::PushAfter => self.push_after = !self.push_after,
            Toggle::SkipHooks => self.skip_hooks = !self.skip_hooks,
        }
    }

    /// Amending starts from the last commit's message when nothing is typed yet,
    /// and warns when that commit is already on a remote.
    fn on_amend_changed(&mut self) {
        let Some(info) = self.loaded_info().cloned() else {
            return;
        };
        let Some(last) = info.last_message.clone() else {
            self.amend = false;
            self.notice = Some(Notice::error(
                "There is no commit to amend yet. Make the first commit without amend.",
            ));
            return;
        };
        let (last_subject, last_body) = last.split_once("\n\n").unwrap_or((&last, ""));
        if self.amend {
            if self.subject.is_empty() && self.body.is_empty() {
                self.subject.set_text(last_subject);
                self.body.set_text(last_body);
            }
            self.notice = Some(if info.head_pushed {
                Notice::error(
                    "The last commit is already pushed. Amending rewrites it, so pushing will need --force.",
                )
            } else {
                Notice::info("Amending the last commit; staged changes are added to it.")
            });
        } else {
            if self.subject.text() == last_subject && self.body.text() == last_body {
                self.subject.set_text("");
                self.body.set_text("");
            }
            self.notice = None;
        }
    }

    pub fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        let control = modifiers.contains(KeyModifiers::CONTROL);
        if control && code == KeyCode::Char('c') {
            self.should_quit = true;
            return;
        }
        if self.phase != Phase::Editing {
            if code == KeyCode::Esc {
                self.notice = Some(Notice::info(
                    "Waiting for git to finish; the screen closes when it is done.",
                ));
            }
            return;
        }
        // Ctrl+Enter arrives as Ctrl+J on many terminals; Ctrl+S always works.
        if control && matches!(code, KeyCode::Enter | KeyCode::Char('s' | 'j')) {
            return self.submit();
        }
        match code {
            KeyCode::Esc => {
                self.exit = Some(CommitExit::Cancelled {
                    subject: self.subject.text(),
                    body: self.body.text(),
                });
                return;
            }
            KeyCode::Tab => {
                self.focus = self.focus.step(1);
                return;
            }
            KeyCode::BackTab => {
                self.focus = self.focus.step(-1);
                return;
            }
            _ => {}
        }
        match self.focus {
            Field::Type => match code {
                KeyCode::Right | KeyCode::Char('t' | ' ') => self.cycle_type(1),
                KeyCode::Left => self.cycle_type(-1),
                KeyCode::Down | KeyCode::Enter => self.focus = Field::Subject,
                _ => {}
            },
            Field::Subject => {
                if !self.subject.handle_key(code, modifiers) {
                    match code {
                        KeyCode::Enter | KeyCode::Down => self.focus = Field::Body,
                        KeyCode::Up => self.focus = Field::Type,
                        _ => {}
                    }
                }
            }
            Field::Body => {
                if !self.body.handle_key(code, modifiers) {
                    match code {
                        KeyCode::Up => self.focus = Field::Subject,
                        KeyCode::Down => self.focus = Field::Options,
                        _ => {}
                    }
                }
            }
            Field::Options => match code {
                KeyCode::Left => self.option = self.option.saturating_sub(1),
                KeyCode::Right => self.option = (self.option + 1).min(Toggle::ORDER.len() - 1),
                KeyCode::Char(' ') | KeyCode::Enter => self.flip(Toggle::ORDER[self.option]),
                KeyCode::Up => self.focus = Field::Body,
                KeyCode::Char(letter) => {
                    if let Some(index) = Toggle::ORDER.iter().position(|t| t.key() == letter) {
                        self.option = index;
                        self.flip(Toggle::ORDER[index]);
                    }
                }
                _ => {}
            },
        }
    }
}

#[cfg(test)]
mod tests;
