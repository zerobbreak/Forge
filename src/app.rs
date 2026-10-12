//! The TUI run loop: terminal setup, background work and routing between screens.

use crate::{
    screens::{
        Screen,
        branches::{self, BranchesView, draw_branches},
        changes::{self, ChangesView, draw_changes},
        commit::{self, CommitExit, CommitView, draw_commit},
        commits::{self, CommitsView, draw_commits},
        projects::{self, ProjectBrowser, draw_browser, projects_root, spawn_loader},
        pulls::{self, PullsView, draw_pulls},
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
    /// The Branches screen, opened from Changes with `b`.
    branches: Option<BranchesView>,
    /// The Pull requests screen, opened from Changes with `4`/`p`.
    pulls: Option<PullsView>,
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
            self.branches = None;
            self.pulls = None;
            return;
        };
        // Esc or a tab key on Commits, Branches or Pull requests ends that screen; a tab
        // key also names the next one, which opens as if Changes had asked for it.
        let wanted = self
            .commits
            .as_ref()
            .and_then(|view| view.goto)
            .or_else(|| self.branches.as_ref().and_then(|view| view.goto))
            .or_else(|| self.pulls.as_ref().and_then(|view| view.goto));
        let closed = self.commits.as_ref().is_some_and(|view| view.closed)
            || self.branches.as_ref().is_some_and(|view| view.closed)
            || self.pulls.as_ref().is_some_and(|view| view.closed);
        if wanted.is_some() || closed {
            // Branches and Pull requests can change the checked-out branch.
            if self.branches.is_some() || self.pulls.is_some() {
                changes.refresh();
            }
            self.commits = None;
            self.branches = None;
            self.pulls = None;
            changes.open_screen = wanted;
        }
        if let Some(screen) = changes.open_screen.take() {
            let (path, tracking) = (changes.path.clone(), changes.tracking());
            match screen {
                Screen::Changes => {}
                Screen::Commits => self.commits = Some(CommitsView::open(path, tracking)),
                Screen::Branches => self.branches = Some(BranchesView::open(path, tracking)),
                Screen::PullRequests => self.pulls = Some(PullsView::open(path, tracking)),
            }
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
            || self.branches.as_ref().is_some_and(|view| view.should_quit)
            || self.pulls.as_ref().is_some_and(|view| view.should_quit)
    }

    fn draw(&self, frame: &mut Frame<'_>) {
        let user = self.browser.user.as_deref();
        match (
            &self.commit,
            &self.commits,
            &self.branches,
            &self.pulls,
            &self.changes,
        ) {
            (Some(view), ..) => draw_commit(frame, view, &self.theme, user),
            (None, Some(view), ..) => draw_commits(frame, view, &self.theme, user),
            (None, None, Some(view), ..) => draw_branches(frame, view, &self.theme, user),
            (None, None, None, Some(view), _) => draw_pulls(frame, view, &self.theme, user),
            (None, None, None, None, Some(view)) => draw_changes(frame, view, &self.theme, user),
            (None, None, None, None, None) => draw_browser(frame, &self.browser, &self.theme),
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
    let (branches_sender, branches_events) = mpsc::channel();
    let (pulls_sender, pulls_events) = mpsc::channel();
    spawn_loader(root.clone(), browser_sender.clone());

    let mut app = App {
        browser: ProjectBrowser::new(root),
        changes: None,
        commit: None,
        commits: None,
        branches: None,
        pulls: None,
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
        while let Ok((path, event)) = branches_events.try_recv() {
            if let Some(view) = app.branches.as_mut().filter(|view| view.path == path) {
                view.apply(event);
            }
        }
        while let Ok((path, event)) = pulls_events.try_recv() {
            if let Some(view) = app.pulls.as_mut().filter(|view| view.path == path) {
                view.apply(event);
            }
        }
        if let Some(view) = &mut app.pulls
            && let Some(request) = view.request.take()
        {
            pulls::spawn_request(view.path.clone(), request, pulls_sender.clone());
        }
        if let Some(view) = &mut app.branches
            && let Some(request) = view.request.take()
        {
            branches::spawn_request(view.path.clone(), request, branches_sender.clone());
        }
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
            match (
                &mut app.commit,
                &mut app.commits,
                &mut app.branches,
                &mut app.pulls,
                &mut app.changes,
            ) {
                (Some(view), ..) => view.handle_key(key.code, key.modifiers),
                (None, Some(view), ..) => view.handle_key(key.code, key.modifiers),
                (None, None, Some(view), ..) => view.handle_key(key.code, key.modifiers),
                (None, None, None, Some(view), _) => view.handle_key(key.code, key.modifiers),
                (None, None, None, None, Some(view)) => view.handle_key(key.code, key.modifiers),
                (None, None, None, None, None) => app.browser.handle_key(key.code, key.modifiers),
            }
        }

        if app.should_quit() {
            break;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::App;
    use crate::{
        screens::projects::{BrowserEvent, ProjectBrowser, load_project},
        test_support::{TestDirectory, run_git},
        theme::Theme,
    };
    use crossterm::event::{KeyCode, KeyModifiers};
    use std::fs;

    /// An app with one real project loaded and opened, as after pressing Enter on it.
    fn app_with_open_project(directory: &TestDirectory) -> App {
        let root = directory.path().to_path_buf();
        let repo = root.join("lumina");
        fs::create_dir_all(&repo).unwrap();
        run_git(&repo, &["init", "-b", "main"]);
        run_git(&repo, &["config", "user.name", "Forge Test"]);
        run_git(&repo, &["config", "user.email", "forge@example.test"]);
        run_git(&repo, &["config", "commit.gpgsign", "false"]);
        fs::write(
            repo.join("a.txt"),
            "1
",
        )
        .unwrap();
        run_git(&repo, &["add", "."]);
        run_git(&repo, &["commit", "-m", "feat: first"]);

        let mut browser = ProjectBrowser::new(root.clone());
        let project = load_project(&root, repo).expect("project loads");
        browser.apply(BrowserEvent::Loaded(project));
        browser.opened_project = Some(0);
        let mut app = App {
            browser,
            changes: None,
            commit: None,
            commits: None,
            branches: None,
            pulls: None,
            theme: Theme::new(false),
        };
        app.route();
        assert!(app.changes.is_some(), "the opened project shows Changes");
        app
    }

    fn open_screens(app: &App) -> (bool, bool, bool) {
        (
            app.commits.is_some(),
            app.branches.is_some(),
            app.pulls.is_some(),
        )
    }

    #[test]
    fn tab_keys_move_between_every_repository_screen() {
        let directory = TestDirectory::new("app-routing");
        let mut app = app_with_open_project(&directory);
        let press = |app: &mut App, code: KeyCode| {
            // Keys go to the topmost open screen, as in the run loop.
            match (
                &mut app.commits,
                &mut app.branches,
                &mut app.pulls,
                &mut app.changes,
            ) {
                (Some(view), ..) => view.handle_key(code, KeyModifiers::NONE),
                (None, Some(view), ..) => view.handle_key(code, KeyModifiers::NONE),
                (None, None, Some(view), _) => view.handle_key(code, KeyModifiers::NONE),
                (None, None, None, Some(view)) => view.handle_key(code, KeyModifiers::NONE),
                _ => unreachable!("a project is open"),
            }
            app.route();
        };

        // Changes -> Pull requests -> Commits -> Branches -> Pull requests -> Changes.
        press(&mut app, KeyCode::Char('4'));
        assert_eq!(open_screens(&app), (false, false, true));
        press(&mut app, KeyCode::Char('2'));
        assert_eq!(open_screens(&app), (true, false, false));
        press(&mut app, KeyCode::Char('3'));
        assert_eq!(open_screens(&app), (false, true, false));
        press(&mut app, KeyCode::Char('4'));
        assert_eq!(open_screens(&app), (false, false, true));
        assert!(
            app.changes.as_ref().unwrap().request.is_some(),
            "leaving Branches makes Changes re-read the repository"
        );
        press(&mut app, KeyCode::Char('1'));
        assert_eq!(open_screens(&app), (false, false, false));

        // The shortcuts from Changes, and Esc back out of each screen.
        for (key, expected) in [
            ('l', (true, false, false)),
            ('b', (false, true, false)),
            ('p', (false, false, true)),
        ] {
            press(&mut app, KeyCode::Char(key));
            assert_eq!(open_screens(&app), expected, "key {key}");
            press(&mut app, KeyCode::Esc);
            assert_eq!(open_screens(&app), (false, false, false), "esc after {key}");
            assert!(app.changes.is_some());
        }

        // Esc on Changes goes back to the project list.
        press(&mut app, KeyCode::Esc);
        assert!(app.changes.is_none());
        assert_eq!(app.browser.opened_project, None);
    }

    #[test]
    fn closing_a_project_closes_the_screens_opened_from_it() {
        let directory = TestDirectory::new("app-routing-close");
        let mut app = app_with_open_project(&directory);
        app.changes
            .as_mut()
            .unwrap()
            .handle_key(KeyCode::Char('3'), KeyModifiers::NONE);
        app.route();
        assert!(app.branches.is_some());
        app.browser.opened_project = None;
        app.route();
        assert!(app.changes.is_none());
        assert_eq!(open_screens(&app), (false, false, false));
    }
}
