//! Shell commands that run outside the TUI: `login`, `projects`, `account`.

use crate::github::{gh_auth_status, list_accounts};
use std::process::{Command, ExitStatus};

pub enum AccountAction {
    List,
    Add,
    Use(String),
}

pub fn login() -> i32 {
    let status = match Command::new("gh").args(["auth", "login"]).status() {
        Ok(status) => status,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("Error: GitHub CLI (`gh`) was not found.");
            eprintln!("Cause: `gh` is not installed or is not on PATH.");
            eprintln!("Next step: Install GitHub CLI and reopen PowerShell.");
            return 1;
        }
        Err(error) => {
            eprintln!("Error: Could not start GitHub CLI.");
            eprintln!("Cause: {error}");
            eprintln!("Next step: Check the GitHub CLI installation and try again.");
            return 1;
        }
    };

    if !status.success() {
        return exit_code(status);
    }

    match gh_auth_status() {
        Ok(()) => {
            println!("Successfully logged in to GitHub using GitHub CLI.");
            0
        }
        Err(cause) => {
            eprintln!("Error: GitHub CLI login could not be verified.");
            eprintln!("Cause: {cause}");
            eprintln!("Next step: Run `gh auth status` and try `forge login` again.");
            1
        }
    }
}

pub fn projects() -> i32 {
    if let Err(cause) = gh_auth_status() {
        eprintln!("Error: Could not fetch GitHub projects.");
        eprintln!("Cause: {cause}");
        eprintln!("Next step: Run `forge login`, then try `forge projects` again.");
        return 1;
    }

    let status = match Command::new("gh")
        .args(["api", "user/repos", "--paginate", "--jq", ".[].full_name"])
        .status()
    {
        Ok(status) => status,
        Err(error) => {
            eprintln!("Error: Could not fetch GitHub projects.");
            eprintln!("Cause: {error}");
            eprintln!("Next step: Check your GitHub CLI installation and try again.");
            return 1;
        }
    };

    exit_code(status)
}

pub fn account(action: AccountAction) -> i32 {
    match action {
        AccountAction::List => account_list(),
        AccountAction::Add => {
            let status = match Command::new("gh")
                .args(["auth", "login", "--hostname", "github.com"])
                .status()
            {
                Ok(status) => status,
                Err(error) => {
                    eprintln!("Error: Could not start GitHub CLI.");
                    eprintln!("Cause: {error}");
                    eprintln!("Next step: Install GitHub CLI and try again.");
                    return 1;
                }
            };

            if !status.success() {
                return exit_code(status);
            }

            account_list()
        }
        AccountAction::Use(username) => {
            let status = match Command::new("gh")
                .args([
                    "auth",
                    "switch",
                    "--hostname",
                    "github.com",
                    "--user",
                    &username,
                ])
                .status()
            {
                Ok(status) => status,
                Err(error) => {
                    eprintln!("Error: Could not switch GitHub accounts.");
                    eprintln!("Cause: {error}");
                    eprintln!("Next step: Check the GitHub CLI installation and try again.");
                    return 1;
                }
            };

            if !status.success() {
                return exit_code(status);
            }

            println!("Active GitHub account: {username}");
            0
        }
    }
}

fn account_list() -> i32 {
    let accounts = match list_accounts() {
        Ok(accounts) => accounts,
        Err(cause) => {
            eprintln!("Error: Could not list GitHub accounts.");
            eprintln!("Cause: {cause}");
            eprintln!("Next step: Run `forge account add` to authenticate with GitHub.");
            return 1;
        }
    };

    if accounts.is_empty() {
        println!("No GitHub.com accounts are configured.");
        println!("Next step: Run `forge account add` to authenticate an account.");
    } else {
        println!("GitHub accounts (active account marked):");
        for account in accounts {
            let status = if account.active { "active" } else { "saved" };
            println!("  {} ({status})", account.login);
        }
        println!("Switch accounts with `forge account use USERNAME`.");
    }

    0
}

fn exit_code(status: ExitStatus) -> i32 {
    status.code().unwrap_or(1)
}
