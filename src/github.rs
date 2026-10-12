//! GitHub access through the GitHub CLI (`gh`).

use std::{io, process::Command};

#[derive(Debug, PartialEq, Eq)]
pub struct Account {
    pub login: String,
    pub active: bool,
}

/// The active GitHub CLI login, shown in the title bar. `None` when it cannot be read.
pub fn active_login() -> Option<String> {
    list_accounts()
        .ok()?
        .into_iter()
        .find(|account| account.active)
        .map(|account| account.login)
}

/// GitHub.com accounts saved in GitHub CLI, in the order `gh` reports them.
pub fn list_accounts() -> Result<Vec<Account>, String> {
    let output = Command::new("gh")
        .args([
            "auth",
            "status",
            "--json",
            "hosts",
            "--jq",
            r#".hosts["github.com"][] | "\(.login)\t\(if .active then "active" else "saved" end)""#,
        ])
        .output()
        .map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                "GitHub CLI (`gh`) is not installed or is not on PATH.".to_owned()
            } else {
                format!("Could not run GitHub CLI: {error}")
            }
        })?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(if detail.is_empty() {
            "GitHub CLI could not read its accounts.".to_owned()
        } else {
            detail
        });
    }
    Ok(parse_accounts(&String::from_utf8_lossy(&output.stdout)))
}

/// Parses `login<TAB>active|saved` lines.
pub fn parse_accounts(text: &str) -> Vec<Account> {
    text.lines()
        .filter_map(|line| line.split_once('\t'))
        .filter(|(login, _)| !login.is_empty())
        .map(|(login, status)| Account {
            login: login.to_owned(),
            active: status.trim() == "active",
        })
        .collect()
}

/// Makes `login` the active GitHub CLI account without printing to the terminal.
pub fn switch_account(login: &str) -> Result<(), String> {
    let output = Command::new("gh")
        .args([
            "auth",
            "switch",
            "--hostname",
            "github.com",
            "--user",
            login,
        ])
        .output()
        .map_err(|error| format!("Could not run GitHub CLI: {error}"))?;
    if output.status.success() {
        return Ok(());
    }
    let detail = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    Err(if detail.is_empty() {
        format!("GitHub CLI could not switch to @{login}.")
    } else {
        detail
    })
}

pub fn gh_auth_status() -> Result<(), String> {
    let output = Command::new("gh")
        .args(["auth", "status"])
        .output()
        .map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                "GitHub CLI (`gh`) is not installed or is not on PATH.".to_owned()
            } else {
                format!("Could not run GitHub CLI: {error}")
            }
        })?;

    if output.status.success() {
        return Ok(());
    }

    let detail = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    if detail.is_empty() {
        Err("GitHub CLI reports no authenticated account.".to_owned())
    } else {
        Err(detail)
    }
}

#[cfg(test)]
mod tests {
    use super::{Account, parse_accounts};

    #[test]
    fn parses_github_cli_account_lines() {
        assert_eq!(
            parse_accounts("octocat\tactive\nhubot\tsaved\n\n"),
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
        );
    }
}
