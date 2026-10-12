//! The TUI run loop: terminal setup, background work and routing between screens.

use crate::{
    screens::{
        Screen,
        branches::{self, BranchesView, draw_branches},
        changes::{self, ChangesView, draw_changes},
        commit::{self, CommitExit, CommitView, draw_commit},
        commits::{self, CommitsView, draw_commits},
        palette::{Action, Outcome, PaletteView, draw_palette},
        projects::{self, ProjectBrowser, draw_browser, projects_root, spawn_loader},
        pulls::{self, PullsView, draw_pulls},
    },
    theme::Theme,
};
use crossterm::{
    cursor::{Hide, Show},
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Frame, Terminal, backend::CrosstermBackend};
use std::{io, sync::mpsc, time::Duration};

/// Reads and clears a request flag set by a screen's key handler.
fn take(flag: &mut bool) -> bool {
    std::mem::take(flag)
}

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
    /// The `/` command palette, drawn over whichever repository screen opened it.
    palette: Option<PaletteView>,
    /// Quit was chosen from the palette.
    quit: bool,
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
            self.palette = None;
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
        self.open_palette_if_asked();
    }

    /// Opens the command palette when the screen on top asked for it with `/`.
    fn open_palette_if_asked(&mut self) {
        if self.palette.is_none()
            && self.wants_palette()
            && let Some(screen) = self.active_screen()
        {
            self.palette = Some(PaletteView::open(screen));
        }
    }

    /// The repository screen on top, or `None` on the project list and the Commit screen.
    fn active_screen(&self) -> Option<Screen> {
        if self.commit.is_some() {
            None
        } else if self.commits.is_some() {
            Some(Screen::Commits)
        } else if self.branches.is_some() {
            Some(Screen::Branches)
        } else if self.pulls.is_some() {
            Some(Screen::PullRequests)
        } else if self.changes.is_some() {
            Some(Screen::Changes)
        } else {
            None
        }
    }

    /// Takes the `/` request from the screen on top, if it made one.
    fn wants_palette(&mut self) -> bool {
        match self.active_screen() {
            Some(Screen::Commits) => self
                .commits
                .as_mut()
                .is_some_and(|v| take(&mut v.open_palette)),
            Some(Screen::Branches) => self
                .branches
                .as_mut()
                .is_some_and(|v| take(&mut v.open_palette)),
            Some(Screen::PullRequests) => self
                .pulls
                .as_mut()
                .is_some_and(|v| take(&mut v.open_palette)),
            Some(Screen::Changes) => self
                .changes
                .as_mut()
                .is_some_and(|v| take(&mut v.open_palette)),
            None => false,
        }
    }

    /// Sends a key to the screen on top: the palette, the Commit screen, a repository
    /// screen, or the project list.
    fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        if let Some(palette) = &mut self.palette {
            palette.handle_key(code, modifiers);
            match palette.outcome.take() {
                Some(Outcome::Close) => self.palette = None,
                Some(Outcome::Run(action)) => {
                    self.palette = None;
                    self.run_action(action);
                }
                None => {}
            }
            return;
        }
        match (
            &mut self.commit,
            &mut self.commits,
            &mut self.branches,
            &mut self.pulls,
            &mut self.changes,
        ) {
            (Some(view), ..) => view.handle_key(code, modifiers),
            (None, Some(view), ..) => view.handle_key(code, modifiers),
            (None, None, Some(view), ..) => view.handle_key(code, modifiers),
            (None, None, None, Some(view), _) => view.handle_key(code, modifiers),
            (None, None, None, None, Some(view)) => view.handle_key(code, modifiers),
            (None, None, None, None, None) => self.browser.handle_key(code, modifiers),
        }
    }

    /// Runs a palette command by pressing the key that does the same thing.
    fn run_action(&mut self, action: Action) {
        match action {
            Action::Quit => self.quit = true,
            Action::Projects => {
                self.commits = None;
                self.branches = None;
                self.pulls = None;
                if let Some(changes) = &mut self.changes {
                    changes.closed = true;
                }
            }
            Action::Screen(screen) => {
                let index = Screen::TABS.iter().position(|(tab, _)| *tab == screen);
                if let Some(digit) = index.and_then(|index| char::from_digit(index as u32 + 1, 10))
                {
                    self.handle_key(KeyCode::Char(digit), KeyModifiers::NONE);
                }
            }
            Action::Key(code) => self.handle_key(code, KeyModifiers::NONE),
        }
    }

    fn should_quit(&self) -> bool {
        self.quit
            || self.browser.should_quit
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
        if let Some(palette) = &self.palette {
            draw_palette(frame, palette, &self.theme);
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
        palette: None,
        quit: false,
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
            app.handle_key(key.code, key.modifiers);
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
            palette: None,
            quit: false,
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

    /// A key press followed by the run loop's routing step.
    fn press(app: &mut App, code: KeyCode) {
        app.handle_key(code, KeyModifiers::NONE);
        app.route();
    }

    fn type_text(app: &mut App, text: &str) {
        for character in text.chars() {
            press(app, KeyCode::Char(character));
        }
    }

    #[test]
    fn tab_keys_move_between_every_repository_screen() {
        let directory = TestDirectory::new("app-routing");
        let mut app = app_with_open_project(&directory);
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

    #[test]
    fn the_palette_opens_with_slash_on_every_repository_screen_and_closes_with_escape() {
        let directory = TestDirectory::new("app-palette-open");
        let mut app = app_with_open_project(&directory);
        for key in ['1', '2', '3', '4'] {
            press(&mut app, KeyCode::Char(key));
            press(&mut app, KeyCode::Char('/'));
            assert!(app.palette.is_some(), "palette on screen {key}");
            press(&mut app, KeyCode::Esc);
            assert!(app.palette.is_none());
            assert!(app.changes.is_some(), "esc closes only the palette");
        }
        // Keys typed into the palette do not reach the screen underneath.
        press(&mut app, KeyCode::Char('/'));
        type_text(&mut app, "q3");
        assert!(!app.should_quit());
        assert!(app.branches.is_none());
        assert_eq!(app.palette.as_ref().unwrap().query(), "q3");
    }

    #[test]
    fn palette_commands_jump_between_screens_by_name() {
        let directory = TestDirectory::new("app-palette-go");
        let mut app = app_with_open_project(&directory);
        press(&mut app, KeyCode::Char('/'));
        type_text(&mut app, "branches");
        press(&mut app, KeyCode::Enter);
        assert!(app.palette.is_none());
        assert_eq!(open_screens(&app), (false, true, false));

        press(&mut app, KeyCode::Char('/'));
        assert_eq!(
            app.palette.as_ref().unwrap().screen,
            crate::screens::Screen::Branches
        );
        type_text(&mut app, "pull");
        press(&mut app, KeyCode::Enter);
        assert_eq!(open_screens(&app), (false, false, true));

        press(&mut app, KeyCode::Char('/'));
        type_text(&mut app, "history");
        press(&mut app, KeyCode::Enter);
        assert_eq!(open_screens(&app), (true, false, false));

        press(&mut app, KeyCode::Char('/'));
        type_text(&mut app, "changes");
        press(&mut app, KeyCode::Enter);
        assert_eq!(open_screens(&app), (false, false, false));
    }

    #[test]
    fn palette_commands_press_the_screens_own_keys() {
        let directory = TestDirectory::new("app-palette-keys");
        let mut app = app_with_open_project(&directory);
        app.changes.as_mut().unwrap().request = None;
        press(&mut app, KeyCode::Char('/'));
        type_text(&mut app, "refresh");
        press(&mut app, KeyCode::Enter);
        assert!(
            app.changes.as_ref().unwrap().request.is_some(),
            "Refresh pressed r on Changes"
        );

        press(&mut app, KeyCode::Char('2'));
        app.commits.as_mut().unwrap().request = None;
        press(&mut app, KeyCode::Char('/'));
        type_text(&mut app, "all branches");
        press(&mut app, KeyCode::Enter);
        assert!(
            app.commits.as_ref().unwrap().request.is_some(),
            "the scope toggle reloads the log"
        );
    }

    #[test]
    fn palette_can_leave_the_project_and_quit() {
        let directory = TestDirectory::new("app-palette-leave");
        let mut app = app_with_open_project(&directory);
        press(&mut app, KeyCode::Char('3'));
        press(&mut app, KeyCode::Char('/'));
        type_text(&mut app, "project list");
        press(&mut app, KeyCode::Enter);
        assert_eq!(open_screens(&app), (false, false, false));
        assert!(app.changes.is_none());
        assert_eq!(app.browser.opened_project, None);

        let mut app = app_with_open_project(&TestDirectory::new("app-palette-quit"));
        press(&mut app, KeyCode::Char('/'));
        type_text(&mut app, "quit");
        press(&mut app, KeyCode::Enter);
        assert!(app.should_quit());
    }

    #[test]
    fn slash_does_not_open_the_palette_on_the_project_list() {
        let directory = TestDirectory::new("app-palette-list");
        let mut app = app_with_open_project(&directory);
        press(&mut app, KeyCode::Esc);
        assert!(app.changes.is_none());
        press(&mut app, KeyCode::Char('/'));
        assert!(app.palette.is_none(), "the list uses / to filter");
    }
}
