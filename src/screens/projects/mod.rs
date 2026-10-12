//! 18 · Projects home: every local repository under the projects folder.

mod view;

pub use view::draw_browser;

use crate::{
    git::repository::{Commit, git_optional_output, git_output, parse_branch_line, parse_log},
    github::{Account, active_login, list_accounts, switch_account},
};
use crossterm::event::{KeyCode, KeyModifiers};
use std::{
    env, fs, io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, mpsc},
    thread,
};

const MAX_DISCOVERY_DEPTH: usize = 8;
const SKIPPED_DIRECTORIES: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    "node_modules",
    "target",
    "vendor",
    ".venv",
    "venv",
    "bin",
    "obj",
    "dist",
    "__pycache__",
];

/// Environment variable that points the project browser at a folder of repositories.
const PROJECTS_DIR_VARIABLE: &str = "FORGE_PROJECTS_DIR";
/// Group heading for repositories that sit directly in the projects folder.
const TOP_LEVEL_GROUP: &str = "LOCAL PROJECTS";
/// Repositories inspected at the same time while the browser loads.
const LOADER_THREADS: usize = 8;

/// Quick filters shown as chips beside the project filter prompt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    All,
    Dirty,
    Behind,
}

impl Scope {
    const ORDER: [Self; 3] = [Self::All, Self::Dirty, Self::Behind];

    fn label(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Dirty => "dirty",
            Self::Behind => "behind",
        }
    }

    fn next(self) -> Self {
        match self {
            Self::All => Self::Dirty,
            Self::Dirty => Self::Behind,
            Self::Behind => Self::All,
        }
    }

    fn includes(self, project: &Project) -> bool {
        match self {
            Self::All => true,
            Self::Dirty => project.dirty_files > 0,
            Self::Behind => project.behind > 0,
        }
    }
}

pub struct ProjectBrowser {
    root: PathBuf,
    pub projects: Vec<Project>,
    selected: usize,
    filter: String,
    filtering: bool,
    scope: Scope,
    pub user: Option<String>,
    loading: bool,
    discovered: Option<usize>,
    unreadable: usize,
    moved: bool,
    account_picker: Option<AccountPicker>,
    pub request: Option<Request>,
    /// The project whose Changes screen is open; the app routes keys there while set.
    pub opened_project: Option<usize>,
    error: Option<String>,
    pub should_quit: bool,
}

impl ProjectBrowser {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            projects: Vec::new(),
            selected: 0,
            filter: String::new(),
            filtering: false,
            scope: Scope::All,
            user: None,
            loading: true,
            discovered: None,
            unreadable: 0,
            moved: false,
            account_picker: None,
            request: None,
            opened_project: None,
            error: None,
            should_quit: false,
        }
    }

    /// Folds a loader event into the browser, keeping the selection on the same project.
    pub fn apply(&mut self, event: BrowserEvent) {
        match event {
            BrowserEvent::User(user) => self.user = user,
            BrowserEvent::Discovered(count) => self.discovered = Some(count),
            BrowserEvent::Loaded(project) => {
                let selected = self.projects.get(self.selected).map(|p| p.path.clone());
                let opened = self
                    .opened_project
                    .and_then(|index| self.projects.get(index))
                    .map(|p| p.path.clone());
                let position = self
                    .projects
                    .partition_point(|existing| project_order(existing, &project).is_lt());
                self.projects.insert(position, project);
                let index_of = |path: &PathBuf| self.projects.iter().position(|p| &p.path == path);
                self.opened_project = opened.as_ref().and_then(index_of);
                // Until the user moves, the cursor stays on the first row as rows stream in.
                match selected.as_ref().and_then(index_of) {
                    Some(index) if self.moved => self.selected = index,
                    _ => self.select_first_visible(),
                }
            }
            BrowserEvent::Unreadable => self.unreadable += 1,
            BrowserEvent::Finished => self.loading = false,
            BrowserEvent::Failed(error) => {
                self.error = Some(error);
                self.loading = false;
            }
            BrowserEvent::Accounts(result) => {
                if let Some(picker) = &mut self.account_picker {
                    match result {
                        Ok(accounts) => {
                            picker.selected = accounts
                                .iter()
                                .position(|account| account.active)
                                .unwrap_or(0);
                            picker.accounts = Some(accounts);
                        }
                        Err(error) => picker.error = Some(error),
                    }
                }
            }
            BrowserEvent::Switched(result) => match result {
                Ok(login) => {
                    self.user = Some(login);
                    self.account_picker = None;
                }
                Err(error) => {
                    if let Some(picker) = &mut self.account_picker {
                        picker.switching = None;
                        picker.error = Some(error);
                    }
                }
            },
        }
    }

    fn visible_projects(&self) -> Vec<usize> {
        let filter = self.filter.to_lowercase();
        self.projects
            .iter()
            .enumerate()
            .filter(|(_, project)| self.scope.includes(project))
            .filter(|(_, project)| {
                filter.is_empty()
                    || project.name.to_lowercase().contains(&filter)
                    || project
                        .path
                        .to_string_lossy()
                        .to_lowercase()
                        .contains(&filter)
                    || project.group.to_lowercase().contains(&filter)
            })
            .map(|(index, _)| index)
            .collect()
    }

    fn needs_attention(&self) -> usize {
        self.projects
            .iter()
            .filter(|project| project.needs_attention())
            .count()
    }

    fn move_selection(&mut self, delta: isize) {
        self.moved = true;
        let visible = self.visible_projects();
        if visible.is_empty() {
            return;
        }
        let current = visible
            .iter()
            .position(|index| *index == self.selected)
            .unwrap_or(0);
        let next = (current as isize + delta).clamp(0, visible.len() as isize - 1) as usize;
        self.selected = visible[next];
    }

    fn select_first_visible(&mut self) {
        if let Some(index) = self.visible_projects().first() {
            self.selected = *index;
        }
    }

    fn jump_to(&mut self, position: usize) {
        self.moved = true;
        if let Some(index) = self.visible_projects().get(position) {
            self.selected = *index;
        }
    }

    fn handle_account_key(&mut self, code: KeyCode) {
        let Some(picker) = &mut self.account_picker else {
            return;
        };
        if picker.switching.is_some() {
            return;
        }
        let count = picker.accounts.as_ref().map_or(0, Vec::len);
        match code {
            KeyCode::Esc => self.account_picker = None,
            KeyCode::Up | KeyCode::Char('k') => picker.selected = picker.selected.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') if picker.selected + 1 < count => {
                picker.selected += 1;
            }
            KeyCode::Enter => {
                let Some(account) = picker
                    .accounts
                    .as_ref()
                    .and_then(|accounts| accounts.get(picker.selected))
                else {
                    return;
                };
                if account.active {
                    self.account_picker = None;
                } else {
                    picker.error = None;
                    picker.switching = Some(account.login.clone());
                    self.request = Some(Request::SwitchAccount(account.login.clone()));
                }
            }
            _ => {}
        }
    }

    pub fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        if (code == KeyCode::Char('q') && !self.filtering)
            || (code == KeyCode::Char('c') && modifiers.contains(KeyModifiers::CONTROL))
        {
            self.should_quit = true;
            return;
        }

        if self.account_picker.is_some() {
            self.handle_account_key(code);
            return;
        }

        if self.filtering {
            match code {
                KeyCode::Enter => self.filtering = false,
                KeyCode::Esc => {
                    self.filtering = false;
                    self.filter.clear();
                    self.select_first_visible();
                }
                KeyCode::Backspace => {
                    self.filter.pop();
                    self.select_first_visible();
                }
                KeyCode::Char(character) => {
                    self.filter.push(character);
                    self.select_first_visible();
                }
                _ => {}
            }
            return;
        }

        if self.opened_project.is_some() {
            if code == KeyCode::Esc {
                self.opened_project = None;
            }
            return;
        }

        match code {
            KeyCode::Up | KeyCode::Char('k') => self.move_selection(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_selection(1),
            KeyCode::Char('/') => self.filtering = true,
            KeyCode::Char('@') => {
                self.account_picker = Some(AccountPicker::default());
                self.request = Some(Request::ListAccounts);
            }
            KeyCode::Tab => {
                self.scope = self.scope.next();
                self.select_first_visible();
            }
            KeyCode::Char(digit @ '1'..='9') => {
                self.jump_to(usize::from(digit as u8 - b'1'));
            }
            KeyCode::Enter if self.visible_projects().contains(&self.selected) => {
                self.opened_project = Some(self.selected)
            }
            _ => {}
        }
    }
}

pub struct Project {
    pub name: String,
    group: String,
    pub path: PathBuf,
    branch: String,
    upstream: bool,
    ahead: usize,
    behind: usize,
    dirty_files: usize,
    remote: Option<String>,
    last_commit: Option<u64>,
    commits: Vec<Commit>,
}

impl Project {
    fn needs_attention(&self) -> bool {
        self.dirty_files > 0 || self.behind > 0
    }
}

pub fn projects_root() -> io::Result<PathBuf> {
    let home = env::var_os("USERPROFILE")
        .or_else(|| env::var_os("HOME"))
        .map(PathBuf::from)
        .ok_or_else(|| io::Error::other("Could not determine the user home directory."))?;
    Ok(resolve_projects_root(
        env::var_os(PROJECTS_DIR_VARIABLE).map(PathBuf::from),
        &home,
        &env::current_dir()?,
    ))
}

/// `FORGE_PROJECTS_DIR`, else `~/projects` when it exists, else the launch folder
/// (unless that is the home folder itself, which is too broad to scan).
fn resolve_projects_root(configured: Option<PathBuf>, home: &Path, current: &Path) -> PathBuf {
    if let Some(configured) = configured.filter(|path| !path.as_os_str().is_empty()) {
        return configured;
    }
    let default = home.join("projects");
    if default.is_dir() || current == home {
        default
    } else {
        current.to_path_buf()
    }
}

/// Progress reported by the background loader to the project browser.
pub enum BrowserEvent {
    User(Option<String>),
    Discovered(usize),
    Loaded(Project),
    Unreadable,
    Finished,
    Failed(String),
    Accounts(Result<Vec<Account>, String>),
    Switched(Result<String, String>),
}

/// Background work the browser asks the run loop to perform, so key handling never blocks.
#[derive(Debug, PartialEq, Eq)]
pub enum Request {
    ListAccounts,
    SwitchAccount(String),
}

pub fn spawn_request(request: Request, sender: mpsc::Sender<BrowserEvent>) {
    thread::spawn(move || {
        let event = match request {
            Request::ListAccounts => BrowserEvent::Accounts(list_accounts()),
            Request::SwitchAccount(login) => {
                BrowserEvent::Switched(switch_account(&login).map(|()| login))
            }
        };
        let _ = sender.send(event);
    });
}

/// The `@` overlay listing GitHub CLI accounts. `accounts` is `None` while loading.
#[derive(Default)]
struct AccountPicker {
    accounts: Option<Vec<Account>>,
    selected: usize,
    switching: Option<String>,
    error: Option<String>,
}

/// Discovers repositories under `root` and inspects them on worker threads, streaming
/// each project to the browser as soon as it is read.
pub fn spawn_loader(root: PathBuf, sender: mpsc::Sender<BrowserEvent>) {
    let user_sender = sender.clone();
    thread::spawn(move || {
        let _ = user_sender.send(BrowserEvent::User(active_login()));
    });

    thread::spawn(move || {
        if !root.is_dir() {
            let _ = sender.send(BrowserEvent::Finished);
            return;
        }
        let mut repositories = Vec::new();
        if let Err(error) = discover_repositories(&root, 0, &mut repositories) {
            let _ = sender.send(BrowserEvent::Failed(format!(
                "Could not scan {}: {error}",
                root.display()
            )));
            return;
        }
        let _ = sender.send(BrowserEvent::Discovered(repositories.len()));

        let queue = Arc::new(Mutex::new(repositories.into_iter()));
        let workers: Vec<_> = (0..LOADER_THREADS)
            .map(|_| {
                let queue = Arc::clone(&queue);
                let root = root.clone();
                let sender = sender.clone();
                thread::spawn(move || {
                    loop {
                        let next = queue.lock().ok().and_then(|mut paths| paths.next());
                        let Some(path) = next else { break };
                        let event = match load_project(&root, path) {
                            Ok(project) => BrowserEvent::Loaded(project),
                            Err(_) => BrowserEvent::Unreadable,
                        };
                        if sender.send(event).is_err() {
                            break;
                        }
                    }
                })
            })
            .collect();
        for worker in workers {
            let _ = worker.join();
        }
        let _ = sender.send(BrowserEvent::Finished);
    });
}

fn discover_repositories(
    directory: &Path,
    depth: usize,
    repositories: &mut Vec<PathBuf>,
) -> io::Result<()> {
    if depth > MAX_DISCOVERY_DEPTH {
        return Ok(());
    }

    let git_metadata = directory.join(".git");
    if git_metadata.is_dir() || git_metadata.is_file() {
        repositories.push(directory.to_path_buf());
        return Ok(());
    }

    // Only the root must be readable; unreadable subfolders are skipped.
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if depth == 0 => return Err(error),
        Err(_) => return Ok(()),
    };
    let mut children = Vec::new();
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !file_type.is_dir()
            || file_type.is_symlink()
            || name.starts_with('.')
            || SKIPPED_DIRECTORIES.contains(&name.as_ref())
        {
            continue;
        }
        children.push(entry.path());
    }
    children.sort();
    for child in children {
        discover_repositories(&child, depth + 1, repositories)?;
    }
    Ok(())
}

#[cfg(test)]
fn load_projects(root: &Path) -> Result<Vec<Project>, String> {
    let mut repositories = Vec::new();
    discover_repositories(root, 0, &mut repositories)
        .map_err(|error| format!("Could not scan {}: {error}", root.display()))?;
    let mut projects = repositories
        .into_iter()
        .map(|path| load_project(root, path))
        .collect::<Result<Vec<_>, _>>()?;
    projects.sort_by(project_order);
    Ok(projects)
}

/// Top-level repositories first, then folder groups alphabetically, then by name.
fn project_order(left: &Project, right: &Project) -> std::cmp::Ordering {
    let key = |project: &Project| {
        (
            project.group != TOP_LEVEL_GROUP,
            project.group.clone(),
            project.name.to_lowercase(),
        )
    };
    key(left).cmp(&key(right))
}

fn load_project(root: &Path, path: PathBuf) -> Result<Project, String> {
    let status = git_output(&path, &["status", "--porcelain=1", "--branch"])?;
    let status_text = String::from_utf8_lossy(&status);
    let mut status_lines = status_text.lines();
    let branch_line = status_lines.next().unwrap_or_default();
    let tracking = parse_branch_line(branch_line);
    let dirty_files = status_lines.filter(|line| !line.trim().is_empty()).count();
    let remote = git_optional_output(&path, &["config", "--get", "remote.origin.url"])?;
    let (last_commit, commits) = if tracking.unborn {
        (None, Vec::new())
    } else {
        parse_log(&String::from_utf8_lossy(&git_output(
            &path,
            &["log", "-3", "--format=%h%x09%ct%x09%s"],
        )?))
    };

    let relative = path.strip_prefix(root).unwrap_or(&path);
    let group = relative
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .and_then(Path::file_name)
        .map(|name| name.to_string_lossy().to_uppercase())
        .unwrap_or_else(|| TOP_LEVEL_GROUP.to_owned());
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());

    Ok(Project {
        name,
        group,
        path,
        branch: tracking.branch,
        upstream: tracking.upstream,
        ahead: tracking.ahead,
        behind: tracking.behind,
        dirty_files,
        remote,
        last_commit,
        commits,
    })
}

#[cfg(test)]
mod tests;
