//! The TUI run loop: terminal setup, background work and routing between screens.

use crate::{
    screens::{
        Screen,
        branches::{self, BranchesEvent, BranchesView, draw_branches},
        changes::{self, ChangesView, draw_changes},
        commit::{self, CommitExit, CommitView, draw_commit},
        commits::{self, CommitsEvent, CommitsView, draw_commits},
        palette::{Action, Outcome, PaletteView, draw_palette},
        projects::{self, ProjectBrowser, draw_browser, projects_root, spawn_loader},
        pulls::{self, PullsEvent, PullsView, draw_pulls},
        search::{self, SearchEvent, SearchView, draw_search},
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
use std::{io, path::PathBuf, sync::mpsc, time::Duration};

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

/// A repository screen shown over Changes. Only one is open at a time.
enum Overlay {
    Commits(CommitsView),
    Branches(BranchesView),
    Pulls(PullsView),
    Search(SearchView),
}

impl Overlay {
    fn screen(&self) -> Screen {
        match self {
            Self::Commits(_) => Screen::Commits,
            Self::Branches(_) => Screen::Branches,
            Self::Pulls(_) => Screen::PullRequests,
            Self::Search(_) => Screen::Search,
        }
    }

    /// The screen a tab key asked for.
    fn goto(&self) -> Option<Screen> {
        match self {
            Self::Commits(view) => view.goto,
            Self::Branches(view) => view.goto,
            Self::Pulls(view) => view.goto,
            Self::Search(view) => view.goto,
        }
    }

    /// Esc, or `1`, asked to go back to Changes.
    fn closed(&self) -> bool {
        match self {
            Self::Commits(view) => view.closed,
            Self::Branches(view) => view.closed,
            Self::Pulls(view) => view.closed,
            Self::Search(view) => view.closed,
        }
    }

    fn should_quit(&self) -> bool {
        match self {
            Self::Commits(view) => view.should_quit,
            Self::Branches(view) => view.should_quit,
            Self::Pulls(view) => view.should_quit,
            Self::Search(view) => view.should_quit,
        }
    }

    /// Whether leaving this screen should make Changes read the repository again.
    fn may_change_repository(&self) -> bool {
        !matches!(self, Self::Commits(_))
    }

    /// Takes the `/` request, if the screen made one.
    fn take_palette_request(&mut self) -> bool {
        match self {
            Self::Commits(view) => take(&mut view.open_palette),
            Self::Branches(view) => take(&mut view.open_palette),
            Self::Pulls(view) => take(&mut view.open_palette),
            Self::Search(view) => take(&mut view.open_palette),
        }
    }

    fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        match self {
            Self::Commits(view) => view.handle_key(code, modifiers),
            Self::Branches(view) => view.handle_key(code, modifiers),
            Self::Pulls(view) => view.handle_key(code, modifiers),
            Self::Search(view) => view.handle_key(code, modifiers),
        }
    }

    fn draw(&self, frame: &mut Frame<'_>, theme: &Theme, user: Option<&str>) {
        match self {
            Self::Commits(view) => draw_commits(frame, view, theme, user),
            Self::Branches(view) => draw_branches(frame, view, theme, user),
            Self::Pulls(view) => draw_pulls(frame, view, theme, user),
            Self::Search(view) => draw_search(frame, view, theme, user),
        }
    }

    /// Hands the screen's pending background work to a worker thread.
    fn start_request(&mut self, senders: &Senders) {
        match self {
            Self::Commits(view) => {
                if let Some(request) = view.request.take() {
                    commits::spawn_request(view.path.clone(), request, senders.commits.clone());
                }
            }
            Self::Branches(view) => {
                if let Some(request) = view.request.take() {
                    branches::spawn_request(view.path.clone(), request, senders.branches.clone());
                }
            }
            Self::Pulls(view) => {
                if let Some(request) = view.request.take() {
                    pulls::spawn_request(view.path.clone(), request, senders.pulls.clone());
                }
            }
            Self::Search(view) => {
                if let Some(request) = view.request.take() {
                    search::spawn_request(view.path.clone(), request, senders.search.clone());
                }
            }
        }
    }
}

/// Where worker threads send each overlay's results.
struct Senders {
    commits: mpsc::Sender<(PathBuf, CommitsEvent)>,
    branches: mpsc::Sender<(PathBuf, BranchesEvent)>,
    pulls: mpsc::Sender<(PathBuf, PullsEvent)>,
    search: mpsc::Sender<(PathBuf, SearchEvent)>,
}

/// The open screens: the project list, plus a project's Changes screen when one is open.
struct App {
    browser: ProjectBrowser,
    changes: Option<ChangesView>,
    /// The Commit screen, opened from Changes with `c`.
    commit: Option<CommitView>,
    /// Commits, Branches, Pull requests or Search, opened from Changes or a tab key.
    overlay: Option<Overlay>,
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
            self.overlay = None;
            self.palette = None;
            return;
        };
        // Esc or a tab key ends the overlay; a tab key also names the next screen, which
        // opens as if Changes had asked for it.
        let wanted = self.overlay.as_ref().and_then(Overlay::goto);
        let closed = self.overlay.as_ref().is_some_and(Overlay::closed);
        if wanted.is_some() || closed {
            if self
                .overlay
                .as_ref()
                .is_some_and(Overlay::may_change_repository)
            {
                changes.refresh();
            }
            self.overlay = None;
            changes.open_screen = wanted;
        }
        if let Some(screen) = changes.open_screen.take() {
            let (path, tracking) = (changes.path.clone(), changes.tracking());
            self.overlay = match screen {
                Screen::Changes => None,
                Screen::Commits => Some(Overlay::Commits(CommitsView::open(path, tracking))),
                Screen::Branches => Some(Overlay::Branches(BranchesView::open(path, tracking))),
                Screen::PullRequests => Some(Overlay::Pulls(PullsView::open(path, tracking))),
                Screen::Search => Some(Overlay::Search(SearchView::open(path, tracking))),
            };
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
        } else if let Some(overlay) = &self.overlay {
            Some(overlay.screen())
        } else {
            self.changes.as_ref().map(|_| Screen::Changes)
        }
    }

    /// Takes the `/` request from the screen on top, if it made one.
    fn wants_palette(&mut self) -> bool {
        if self.commit.is_some() {
            return false;
        }
        match (&mut self.overlay, &mut self.changes) {
            (Some(overlay), _) => overlay.take_palette_request(),
            (None, Some(changes)) => take(&mut changes.open_palette),
            (None, None) => false,
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
        match (&mut self.commit, &mut self.overlay, &mut self.changes) {
            (Some(view), ..) => view.handle_key(code, modifiers),
            (None, Some(overlay), _) => overlay.handle_key(code, modifiers),
            (None, None, Some(view)) => view.handle_key(code, modifiers),
            (None, None, None) => self.browser.handle_key(code, modifiers),
        }
    }

    /// Runs a palette command by pressing the key that does the same thing.
    fn run_action(&mut self, action: Action) {
        match action {
            Action::Quit => self.quit = true,
            Action::Projects => {
                self.overlay = None;
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
            || self.overlay.as_ref().is_some_and(Overlay::should_quit)
    }

    fn draw(&self, frame: &mut Frame<'_>) {
        let user = self.browser.user.as_deref();
        match (&self.commit, &self.overlay, &self.changes) {
            (Some(view), ..) => draw_commit(frame, view, &self.theme, user),
            (None, Some(overlay), _) => overlay.draw(frame, &self.theme, user),
            (None, None, Some(view)) => draw_changes(frame, view, &self.theme, user),
            (None, None, None) => draw_browser(frame, &self.browser, &self.theme),
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
    let (search_sender, search_events) = mpsc::channel();
    let senders = Senders {
        commits: commits_sender,
        branches: branches_sender,
        pulls: pulls_sender,
        search: search_sender,
    };
    spawn_loader(root.clone(), browser_sender.clone());

    let mut app = App {
        browser: ProjectBrowser::new(root),
        changes: None,
        commit: None,
        overlay: None,
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
            // Results for a screen that has since closed are dropped.
            if let Some(view) = app.changes.as_mut().filter(|view| view.path == path) {
                view.apply(event);
            }
        }
        while let Ok((path, event)) = commit_events.try_recv() {
            if let Some(view) = app.commit.as_mut().filter(|view| view.path == path) {
                view.apply(event);
            }
        }
        while let Ok((path, event)) = commits_events.try_recv() {
            if let Some(Overlay::Commits(view)) = &mut app.overlay
                && view.path == path
            {
                view.apply(event);
            }
        }
        while let Ok((path, event)) = branches_events.try_recv() {
            if let Some(Overlay::Branches(view)) = &mut app.overlay
                && view.path == path
            {
                view.apply(event);
            }
        }
        while let Ok((path, event)) = pulls_events.try_recv() {
            if let Some(Overlay::Pulls(view)) = &mut app.overlay
                && view.path == path
            {
                view.apply(event);
            }
        }
        while let Ok((path, event)) = search_events.try_recv() {
            if let Some(Overlay::Search(view)) = &mut app.overlay
                && view.path == path
            {
                view.apply(event);
            }
        }
        app.route();
        if let Some(view) = &mut app.changes
            && let Some(request) = view.request.take()
        {
            changes::spawn_request(view.path.clone(), request, changes_sender.clone());
        }
        if let Some(overlay) = &mut app.overlay {
            overlay.start_request(&senders);
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
    use super::{App, Overlay};
    use crate::{
        screens::{
            Screen,
            projects::{BrowserEvent, ProjectBrowser, load_project},
        },
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
        fs::write(repo.join("a.txt"), "1\n").unwrap();
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
            overlay: None,
            palette: None,
            quit: false,
            theme: Theme::new(false),
        };
        app.route();
        assert!(app.changes.is_some(), "the opened project shows Changes");
        app
    }

    /// The overlay on top, if any.
    fn open_screen(app: &App) -> Option<Screen> {
        app.overlay.as_ref().map(Overlay::screen)
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

        // Changes -> every screen by number, each jumping straight to the next.
        for (digit, screen) in [
            ('4', Screen::PullRequests),
            ('2', Screen::Commits),
            ('5', Screen::Search),
            ('3', Screen::Branches),
            ('4', Screen::PullRequests),
        ] {
            // Search starts in a text box, where digits are text: move to the results first.
            if open_screen(&app) == Some(Screen::Search) {
                press(&mut app, KeyCode::Tab);
                press(&mut app, KeyCode::Tab);
            }
            press(&mut app, KeyCode::Char(digit));
            assert_eq!(open_screen(&app), Some(screen), "key {digit}");
        }
        assert!(
            app.changes.as_ref().unwrap().request.is_some(),
            "leaving Branches makes Changes re-read the repository"
        );
        press(&mut app, KeyCode::Char('1'));
        assert_eq!(open_screen(&app), None);

        // The shortcuts from Changes, and Esc back out of each screen.
        for (key, expected) in [
            ('l', Screen::Commits),
            ('b', Screen::Branches),
            ('p', Screen::PullRequests),
            ('f', Screen::Search),
        ] {
            press(&mut app, KeyCode::Char(key));
            assert_eq!(open_screen(&app), Some(expected), "key {key}");
            press(&mut app, KeyCode::Esc);
            assert_eq!(open_screen(&app), None, "esc after {key}");
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
        press(&mut app, KeyCode::Char('3'));
        assert_eq!(open_screen(&app), Some(Screen::Branches));
        app.browser.opened_project = None;
        app.route();
        assert!(app.changes.is_none());
        assert_eq!(open_screen(&app), None);
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
        assert_ne!(open_screen(&app), Some(Screen::Branches));
        assert_eq!(app.palette.as_ref().unwrap().query(), "q3");
    }

    #[test]
    fn palette_commands_jump_between_screens_by_name() {
        let directory = TestDirectory::new("app-palette-go");
        let mut app = app_with_open_project(&directory);
        for (query, screen) in [
            ("branches", Some(Screen::Branches)),
            ("pull", Some(Screen::PullRequests)),
            ("history", Some(Screen::Commits)),
            ("grep", Some(Screen::Search)),
        ] {
            press(&mut app, KeyCode::Char('/'));
            type_text(&mut app, query);
            press(&mut app, KeyCode::Enter);
            assert!(app.palette.is_none());
            assert_eq!(open_screen(&app), screen, "{query}");
            if screen == Some(Screen::Search) {
                break;
            }
        }

        // From Search's results, the palette can go back to Changes.
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Char('/'));
        assert_eq!(
            app.palette.as_ref().unwrap().screen,
            Screen::Search,
            "the palette lists Search's own commands"
        );
        type_text(&mut app, "go to changes");
        press(&mut app, KeyCode::Enter);
        assert_eq!(open_screen(&app), None);
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
        if let Some(Overlay::Commits(view)) = &mut app.overlay {
            view.request = None;
        }
        press(&mut app, KeyCode::Char('/'));
        type_text(&mut app, "all branches");
        press(&mut app, KeyCode::Enter);
        let Some(Overlay::Commits(view)) = &app.overlay else {
            panic!("still on Commits");
        };
        assert!(view.request.is_some(), "the scope toggle reloads the log");
    }

    #[test]
    fn palette_can_leave_the_project_and_quit() {
        let directory = TestDirectory::new("app-palette-leave");
        let mut app = app_with_open_project(&directory);
        press(&mut app, KeyCode::Char('3'));
        press(&mut app, KeyCode::Char('/'));
        type_text(&mut app, "project list");
        press(&mut app, KeyCode::Enter);
        assert_eq!(open_screen(&app), None);
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

    #[test]
    fn slash_is_text_in_the_search_boxes_and_a_palette_request_in_the_results() {
        let directory = TestDirectory::new("app-search-slash");
        let mut app = app_with_open_project(&directory);
        press(&mut app, KeyCode::Char('f'));
        assert_eq!(open_screen(&app), Some(Screen::Search));
        type_text(&mut app, "src/main");
        assert!(app.palette.is_none(), "paths contain slashes");
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Char('/'));
        assert!(app.palette.is_some());
    }
}
