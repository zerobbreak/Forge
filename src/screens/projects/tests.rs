use super::{
    BrowserEvent, Project, ProjectBrowser, Request, Scope, TOP_LEVEL_GROUP, draw_browser,
    load_project, load_projects, resolve_projects_root,
};
use crate::{
    git::repository::Commit,
    github::Account,
    test_support::{TestDirectory, run_git},
    theme::Theme,
};
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::{Terminal, backend::TestBackend};
use std::{fs, path::PathBuf};

fn project(name: &str, group: &str) -> Project {
    Project {
        name: name.to_owned(),
        group: group.to_owned(),
        path: PathBuf::from(format!("C:\\projects\\{group}\\{name}")),
        branch: "main".to_owned(),
        upstream: true,
        ahead: 0,
        behind: 0,
        dirty_files: 0,
        remote: Some("https://github.com/example/repo".to_owned()),
        last_commit: None,
        commits: vec![Commit {
            hash: "abc1234".to_owned(),
            subject: "initial commit".to_owned(),
        }],
    }
}

fn loaded_browser(projects: Vec<Project>) -> ProjectBrowser {
    let mut app = ProjectBrowser::new(PathBuf::from("C:\\projects"));
    app.projects = projects;
    app.loading = false;
    app.user = Some("octocat".to_owned());
    app
}

fn render_browser(width: u16, height: u16, app: &ProjectBrowser) -> String {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("test terminal should initialize");
    terminal
        .draw(|frame| draw_browser(frame, app, &Theme::new(true)))
        .expect("browser should render");
    terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}

#[test]
fn supports_quit_keys_in_tui() {
    let mut app = ProjectBrowser::new(PathBuf::from("C:\\projects"));
    app.handle_key(KeyCode::Char('q'), KeyModifiers::NONE);
    assert!(app.should_quit);

    let mut app = ProjectBrowser::new(PathBuf::from("C:\\projects"));
    app.handle_key(KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert!(app.should_quit);

    let mut app = ProjectBrowser::new(PathBuf::from("C:\\projects"));
    app.opened_project = Some(0);
    app.handle_key(KeyCode::Esc, KeyModifiers::NONE);
    assert_eq!(app.opened_project, None);
    assert!(!app.should_quit);
}

#[test]
fn lists_selected_project_and_details_at_80x24() {
    let mut dirty = project("lumina", "WORK");
    dirty.dirty_files = 4;
    dirty.ahead = 3;
    let app = loaded_browser(vec![project("forge", "WORK"), dirty]);

    let rendered = render_browser(80, 24, &app);
    assert!(rendered.contains("FORGE"));
    assert!(rendered.contains("2 repositories · 1 need attention"));
    assert!(rendered.contains("@octocat"));
    assert!(rendered.contains("> 1 forge"));
    assert!(rendered.contains("2 lumina"));
    assert!(rendered.contains("↑3 ↓0"));
    assert!(rendered.contains("RECENT COMMITS"));
    assert!(rendered.contains("enter  Open"));
}

#[test]
fn lists_selected_project_and_details_at_wide_size() {
    let app = loaded_browser(vec![project("forge", "WORK")]);

    let rendered = render_browser(120, 40, &app);
    assert!(rendered.contains("WORK"));
    assert!(rendered.contains("[all]"));
    assert!(rendered.contains("⎇ main"));
    assert!(rendered.contains("clean"));
    assert!(rendered.contains("initial commit"));
    assert!(rendered.contains("OPEN WITH"));
}

#[test]
fn shows_scanning_state_before_projects_load() {
    let app = ProjectBrowser::new(PathBuf::from("C:\\projects"));

    let rendered = render_browser(80, 24, &app);
    assert!(rendered.contains("scanning…"));
    assert!(rendered.contains("Scanning local repositories"));
}

#[test]
fn filter_navigation_and_open_project_work() {
    let mut app = loaded_browser(vec![project("alpha", "WORK"), project("qbeta", "PERSONAL")]);
    app.handle_key(KeyCode::Down, KeyModifiers::NONE);
    assert_eq!(app.selected, 1);

    app.handle_key(KeyCode::Char('/'), KeyModifiers::NONE);
    app.handle_key(KeyCode::Char('q'), KeyModifiers::NONE);
    assert!(!app.should_quit);
    assert_eq!(app.visible_projects(), vec![1]);

    app.handle_key(KeyCode::Backspace, KeyModifiers::NONE);
    assert_eq!(app.visible_projects(), vec![0, 1]);
    app.handle_key(KeyCode::Enter, KeyModifiers::NONE);
    app.handle_key(KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(app.opened_project, Some(0));
    app.handle_key(KeyCode::Esc, KeyModifiers::NONE);
    assert_eq!(app.opened_project, None);
}

#[test]
fn escape_clears_filter() {
    let mut app = loaded_browser(vec![project("alpha", "WORK"), project("beta", "WORK")]);
    app.handle_key(KeyCode::Char('/'), KeyModifiers::NONE);
    app.handle_key(KeyCode::Char('b'), KeyModifiers::NONE);
    assert_eq!(app.visible_projects(), vec![1]);

    app.handle_key(KeyCode::Esc, KeyModifiers::NONE);
    assert!(!app.filtering);
    assert_eq!(app.visible_projects(), vec![0, 1]);
}

#[test]
fn number_keys_jump_to_visible_position() {
    let mut app = loaded_browser(vec![
        project("alpha", "WORK"),
        project("beta", "WORK"),
        project("gamma", "WORK"),
    ]);
    app.handle_key(KeyCode::Char('3'), KeyModifiers::NONE);
    assert_eq!(app.selected, 2);
    app.handle_key(KeyCode::Char('9'), KeyModifiers::NONE);
    assert_eq!(app.selected, 2);
}

#[test]
fn tab_cycles_scope_through_dirty_and_behind() {
    let mut dirty = project("dirty", "WORK");
    dirty.dirty_files = 2;
    let mut behind = project("behind", "WORK");
    behind.behind = 1;
    let mut app = loaded_browser(vec![project("clean", "WORK"), dirty, behind]);

    app.handle_key(KeyCode::Tab, KeyModifiers::NONE);
    assert_eq!(app.scope, Scope::Dirty);
    assert_eq!(app.visible_projects(), vec![1]);
    assert_eq!(app.selected, 1);

    app.handle_key(KeyCode::Tab, KeyModifiers::NONE);
    assert_eq!(app.scope, Scope::Behind);
    assert_eq!(app.visible_projects(), vec![2]);

    app.handle_key(KeyCode::Tab, KeyModifiers::NONE);
    assert_eq!(app.scope, Scope::All);
    assert_eq!(app.visible_projects(), vec![0, 1, 2]);
}

#[test]
fn enter_does_not_open_project_hidden_by_scope() {
    let mut app = loaded_browser(vec![project("clean", "WORK")]);
    app.handle_key(KeyCode::Tab, KeyModifiers::NONE);
    app.handle_key(KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(app.opened_project, None);
}

#[test]
fn streamed_projects_keep_the_current_selection() {
    let mut app = ProjectBrowser::new(PathBuf::from("C:\\projects"));
    app.apply(BrowserEvent::Discovered(6));
    app.apply(BrowserEvent::Loaded(project("beta", "WORK")));
    assert_eq!(app.projects[app.selected].name, "beta");

    app.apply(BrowserEvent::Loaded(project("delta", "WORK")));
    app.apply(BrowserEvent::Loaded(project("alpha", "WORK")));
    assert_eq!(app.projects[0].name, "alpha");
    assert_eq!(app.projects[app.selected].name, "alpha");

    app.handle_key(KeyCode::Down, KeyModifiers::NONE);
    app.apply(BrowserEvent::Loaded(project("aardvark", "WORK")));
    assert_eq!(app.projects[app.selected].name, "beta");

    app.apply(BrowserEvent::Loaded(project("top", TOP_LEVEL_GROUP)));
    assert_eq!(app.projects[0].name, "top");

    app.apply(BrowserEvent::Unreadable);
    let rendered = render_browser(80, 24, &app);
    assert!(rendered.contains("reading 6/6"));
    assert!(rendered.contains("1 unreadable"));

    app.apply(BrowserEvent::Finished);
    assert!(!app.loading);
    assert!(render_browser(80, 24, &app).contains("5 repositories"));
}

#[test]
fn resolves_projects_root_from_setting_default_or_launch_folder() {
    let test_dir = TestDirectory::new("projects-root");
    let home = test_dir.path().join("home");
    let launch = home.join("code");
    fs::create_dir_all(&launch).expect("launch directory should be created");

    let configured = PathBuf::from("D:\\repos");
    assert_eq!(
        resolve_projects_root(Some(configured.clone()), &home, &launch),
        configured
    );
    assert_eq!(resolve_projects_root(None, &home, &launch), launch);
    assert_eq!(
        resolve_projects_root(Some(PathBuf::new()), &home, &home),
        home.join("projects")
    );

    fs::create_dir_all(home.join("projects")).expect("projects directory should be created");
    assert_eq!(
        resolve_projects_root(None, &home, &launch),
        home.join("projects")
    );
}

fn accounts() -> Vec<Account> {
    vec![
        Account {
            login: "octocat".to_owned(),
            active: true,
        },
        Account {
            login: "hubot".to_owned(),
            active: false,
        },
    ]
}

#[test]
fn at_key_opens_account_picker_and_switches_in_background() {
    let mut app = loaded_browser(vec![project("forge", "WORK")]);
    app.handle_key(KeyCode::Char('@'), KeyModifiers::NONE);
    assert!(app.account_picker.is_some());
    assert_eq!(app.request.take(), Some(Request::ListAccounts));
    assert!(render_browser(80, 24, &app).contains("Reading GitHub CLI accounts"));

    app.apply(BrowserEvent::Accounts(Ok(accounts())));
    let picker = app.account_picker.as_ref().expect("picker stays open");
    assert_eq!(picker.selected, 0);

    app.handle_key(KeyCode::Down, KeyModifiers::NONE);
    app.handle_key(KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(
        app.request.take(),
        Some(Request::SwitchAccount("hubot".to_owned()))
    );
    assert!(render_browser(80, 24, &app).contains("switching…"));

    // Keys are ignored while the switch is in flight.
    app.handle_key(KeyCode::Esc, KeyModifiers::NONE);
    assert!(app.account_picker.is_some());

    app.apply(BrowserEvent::Switched(Ok("hubot".to_owned())));
    assert!(app.account_picker.is_none());
    assert_eq!(app.user.as_deref(), Some("hubot"));
    assert!(render_browser(80, 24, &app).contains("@hubot"));
}

#[test]
fn account_picker_shows_errors_and_closes_on_escape() {
    let mut app = loaded_browser(vec![project("forge", "WORK")]);
    app.handle_key(KeyCode::Char('@'), KeyModifiers::NONE);
    app.apply(BrowserEvent::Accounts(Ok(accounts())));
    app.handle_key(KeyCode::Down, KeyModifiers::NONE);
    app.handle_key(KeyCode::Enter, KeyModifiers::NONE);
    app.apply(BrowserEvent::Switched(Err("no such account".to_owned())));

    let rendered = render_browser(80, 24, &app);
    assert!(rendered.contains("SWITCH ACCOUNT"));
    assert!(rendered.contains("no such account"));
    assert_eq!(app.user.as_deref(), Some("octocat"));

    app.handle_key(KeyCode::Esc, KeyModifiers::NONE);
    assert!(app.account_picker.is_none());
    assert!(!app.should_quit);
}

#[test]
fn enter_on_active_account_just_closes_picker() {
    let mut app = loaded_browser(vec![project("forge", "WORK")]);
    app.handle_key(KeyCode::Char('@'), KeyModifiers::NONE);
    app.request = None;
    app.apply(BrowserEvent::Accounts(Ok(accounts())));
    let rendered = render_browser(120, 40, &app);
    assert!(rendered.contains("@octocat"));
    assert!(rendered.contains("active"));

    app.handle_key(KeyCode::Enter, KeyModifiers::NONE);
    assert!(app.account_picker.is_none());
    assert_eq!(app.request, None);
}

#[test]
fn reports_ahead_and_behind_against_upstream() {
    let test_dir = TestDirectory::new("ahead-behind");
    let upstream = test_dir.path().join("upstream");
    fs::create_dir_all(&upstream).expect("upstream directory should be created");
    run_git(&upstream, &["init"]);
    run_git(&upstream, &["config", "user.name", "Forge Test"]);
    run_git(&upstream, &["config", "user.email", "forge@example.test"]);
    fs::write(upstream.join("a.txt"), "a").expect("file should be written");
    run_git(&upstream, &["add", "a.txt"]);
    run_git(&upstream, &["commit", "-m", "base"]);

    let root = test_dir.path().join("projects");
    let clone = root.join("clone");
    fs::create_dir_all(&root).expect("projects directory should be created");
    run_git(
        &root,
        &["clone", upstream.to_str().expect("utf-8 path"), "clone"],
    );
    run_git(&clone, &["config", "user.name", "Forge Test"]);
    run_git(&clone, &["config", "user.email", "forge@example.test"]);
    fs::write(clone.join("b.txt"), "b").expect("file should be written");
    run_git(&clone, &["add", "b.txt"]);
    run_git(&clone, &["commit", "-m", "local"]);
    fs::write(upstream.join("c.txt"), "c").expect("file should be written");
    run_git(&upstream, &["add", "c.txt"]);
    run_git(&upstream, &["commit", "-m", "remote"]);
    run_git(&clone, &["fetch"]);

    let loaded = load_project(&root, clone).expect("clone should load");
    assert!(loaded.upstream);
    assert_eq!((loaded.ahead, loaded.behind), (1, 1));
    assert!(loaded.needs_attention());
}

#[test]
fn loads_branch_dirty_state_remote_and_recent_commits_from_local_repo() {
    let test_dir = TestDirectory::new("load-project");
    let root = test_dir.path().join("projects");
    let group = root.join("work");
    let repository = group.join("demo");
    fs::create_dir_all(&repository).expect("repository directory should be created");
    run_git(&repository, &["init"]);
    run_git(&repository, &["config", "user.name", "Forge Test"]);
    run_git(&repository, &["config", "user.email", "forge@example.test"]);
    run_git(
        &repository,
        &["remote", "add", "origin", "https://example.test/demo.git"],
    );
    fs::write(repository.join("tracked.txt"), "content").expect("tracked file should be written");
    run_git(&repository, &["add", "tracked.txt"]);
    run_git(&repository, &["commit", "-m", "first commit"]);
    fs::write(repository.join("untracked.txt"), "local").expect("untracked file should be written");

    let loaded = load_project(&root, repository).expect("project metadata should load");
    assert_eq!(loaded.name, "demo");
    assert_eq!(loaded.group, "WORK");
    assert!(!loaded.branch.is_empty());
    assert_eq!(loaded.dirty_files, 1);
    assert_eq!(
        loaded.remote.as_deref(),
        Some("https://example.test/demo.git")
    );
    assert_eq!(loaded.commits.len(), 1);
    assert_eq!(loaded.commits[0].subject, "first commit");
    assert!(loaded.last_commit.is_some());
}

#[test]
fn handles_unborn_repository_without_recent_commits() {
    let test_dir = TestDirectory::new("unborn-project");
    let root = test_dir.path().join("projects");
    let repository = root.join("new-repo");
    fs::create_dir_all(&repository).expect("repository directory should be created");
    run_git(&repository, &["init"]);

    let loaded = load_project(&root, repository).expect("empty repo should load");
    assert_eq!(loaded.commits.len(), 0);
    assert!(!loaded.branch.is_empty());
}

#[test]
fn discovers_nested_repositories_and_skips_generated_folders() {
    let test_dir = TestDirectory::new("discover");
    let root = test_dir.path().join("projects");
    let repository = root.join("work").join("demo");
    fs::create_dir_all(&repository).expect("repository directory should be created");
    run_git(&repository, &["init"]);

    let generated_repository = root.join("target").join("generated");
    fs::create_dir_all(generated_repository.join(".git"))
        .expect("generated directory should be created");

    let projects = load_projects(&root).expect("projects should be discovered");
    assert_eq!(projects.len(), 1);
    assert_eq!(projects[0].name, "demo");
    assert_eq!(projects[0].group, "WORK");
}
