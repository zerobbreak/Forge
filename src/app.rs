//! The TUI run loop: terminal setup, background work and routing between screens.

use crate::{
    screens::{
        changes::{self, ChangesView, draw_changes},
        commit::{self, CommitExit, CommitView, draw_commit},
        commits::{self, CommitsView, draw_commits},
        projects::{self, ProjectBrowser, draw_browser, projects_root, spawn_loader},
    },
    theme::Theme,
};
use crossterm::{
    cursor::{Hide, Show},
    event::{self, Event, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Frame, Terminal, backend::CrosstermBackend};
use std::{io, sync::mpsc, time::Duration};

/// Restores the terminal when dropped, including while unwinding from a panic.
struct TerminalGuard;

impl TerminalGuard {
    fn enter() -> io::Result<Self> {
        enable_raw_mode()?;
        if let Err(error) = execute!(io::stdout(), EnterAlternateScreen, Hide) {
            let _ = execute!(io::stdout(), Show, LeaveAlternateScreen);
            let _ = disable_raw_mode();
            return Err(error);
        }
        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = execute!(io::stdout(), Show, LeaveAlternateScreen);
        let _ = disable_raw_mode();
    }
}

/// The open screens: the project list, plus a project's Changes screen when one is open.
struct App {
    browser: ProjectBrowser,
    changes: Option<ChangesView>,
    /// The Commit screen, opened from Changes with `c`.
    commit: Option<CommitView>,
    /// The Commits (history) screen, opened from Changes with `l`.
    commits: Option<CommitsView>,
    theme: Theme,
}

impl App {
    /// Opens or closes the Changes screen to match the browser's opened project.
    fn route(&mut self) {
        if let Some(view) = &self.changes
            && view.closed
        {
            self.changes = None;
            self.browser.opened_project = None;
        }
        let opened = self
            .browser
            .opened_project
            .and_then(|index| self.browser.projects.get(index))
            .map(|project| project.path.clone());
        match (opened, &self.changes) {
            (Some(path), Some(view)) if view.path == path => {}
            (Some(path), _) => self.changes = Some(ChangesView::open(path)),
            (None, _) => self.changes = None,
        }

        let Some(changes) = &mut self.changes else {
            self.commit = None;
            self.commits = None;
            return;
        };
        if self.commits.as_ref().is_some_and(|view| view.closed) {
            self.commits = None;
        }
        if changes.open_commits {
            changes.open_commits = false;
            self.commits = Some(CommitsView::open(changes.path.clone(), changes.tracking()));
        }
        if let Some(exit) = self.commit.as_mut().and_then(|view| view.exit.take()) {
            self.commit = None;
            match exit {
                CommitExit::Cancelled { subject, body } => {
                    let typed = !subject.is_empty() || !body.is_empty();
                    changes.draft = typed.then_some((subject, body));
                }
                CommitExit::Done(notice) => changes.commit_finished(notice),
            }
        }
        if changes.open_commit {
            changes.open_commit = false;
            let (staged, tracking) = changes.commit_context();
            self.commit = Some(CommitView::open(
                changes.path.clone(),
                staged,
                tracking,
                changes.draft.take(),
            ));
        }
    }

    fn should_quit(&self) -> bool {
        self.browser.should_quit
            || self.changes.as_ref().is_some_and(|view| view.should_quit)
            || self.commit.as_ref().is_some_and(|view| view.should_quit)
            || self.commits.as_ref().is_some_and(|view| view.should_quit)
    }

    fn draw(&self, frame: &mut Frame<'_>) {
        let user = self.browser.user.as_deref();
        match (&self.commit, &self.commits, &self.changes) {
            (Some(view), _, _) => draw_commit(frame, view, &self.theme, user),
            (None, Some(view), _) => draw_commits(frame, view, &self.theme, user),
            (None, None, Some(view)) => draw_changes(frame, view, &self.theme, user),
            (None, None, None) => draw_browser(frame, &self.browser, &self.theme),
        }
    }
}

pub fn run_tui() -> io::Result<()> {
    let _terminal = TerminalGuard::enter()?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    let root = projects_root()?;
    let (browser_sender, browser_events) = mpsc::channel();
    let (changes_sender, changes_events) = mpsc::channel();
    let (commit_sender, commit_events) = mpsc::channel();
    let (commits_sender, commits_events) = mpsc::channel();
    spawn_loader(root.clone(), browser_sender.clone());

    let mut app = App {
        browser: ProjectBrowser::new(root),
        changes: None,
        commit: None,
        commits: None,
        theme: Theme::from_env(),
    };
    loop {
        while let Ok(event) = browser_events.try_recv() {
            app.browser.apply(event);
        }
        app.route();
        while let Ok((path, event)) = changes_events.try_recv() {
            // Results for a Changes screen that has since closed are dropped.
            if let Some(view) = app.changes.as_mut().filter(|view| view.path == path) {
                view.apply(event);
            }
        }
        if let Some(view) = &mut app.changes
            && let Some(request) = view.request.take()
        {
            changes::spawn_request(view.path.clone(), request, changes_sender.clone());
        }
        while let Ok((path, event)) = commit_events.try_recv() {
            if let Some(view) = app.commit.as_mut().filter(|view| view.path == path) {
                view.apply(event);
            }
        }
        while let Ok((path, event)) = commits_events.try_recv() {
            if let Some(view) = app.commits.as_mut().filter(|view| view.path == path) {
                view.apply(event);
            }
        }
        app.route();
        if let Some(view) = &mut app.commits
            && let Some(request) = view.request.take()
        {
            commits::spawn_request(view.path.clone(), request, commits_sender.clone());
        }
        if let Some(view) = &mut app.commit
            && let Some(request) = view.request.take()
        {
            commit::spawn_request(view.path.clone(), request, commit_sender.clone());
        }
        if let Some(request) = app.browser.request.take() {
            projects::spawn_request(request, browser_sender.clone());
        }

        terminal.draw(|frame| app.draw(frame))?;

        if event::poll(Duration::from_millis(100))?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            match (&mut app.commit, &mut app.commits, &mut app.changes) {
                (Some(view), _, _) => view.handle_key(key.code, key.modifiers),
                (None, Some(view), _) => view.handle_key(key.code, key.modifiers),
                (None, None, Some(view)) => view.handle_key(key.code, key.modifiers),
                (None, None, None) => app.browser.handle_key(key.code, key.modifiers),
            }
        }

        if app.should_quit() {
            break;
        }
    }

    Ok(())
}
