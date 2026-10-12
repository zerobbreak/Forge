mod app;
mod cli;
mod editor;
mod git;
mod github;
mod screens;
#[cfg(test)]
mod test_support;
mod theme;
mod ui;

use cli::AccountAction;

enum Action {
    Start,
    Login,
    Projects,
    Account(AccountAction),
    Help,
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    std::process::exit(run(&args));
}

fn parse_args(args: &[String]) -> Result<Action, String> {
    match args {
        [] => Ok(Action::Start),
        [command] if command == "login" => Ok(Action::Login),
        [command] if command == "projects" => Ok(Action::Projects),
        [command, subcommand] if command == "account" && subcommand == "list" => {
            Ok(Action::Account(AccountAction::List))
        }
        [command, subcommand] if command == "account" && subcommand == "add" => {
            Ok(Action::Account(AccountAction::Add))
        }
        [command, subcommand, username]
            if command == "account" && subcommand == "use" && !username.is_empty() =>
        {
            Ok(Action::Account(AccountAction::Use(username.clone())))
        }
        [command] if command == "--help" || command == "-h" => Ok(Action::Help),
        [command] => Err(format!("Unknown command: {command}")),
        [command, ..] if command == "account" => {
            Err("Usage: forge account <list|add|use USERNAME>".to_owned())
        }
        _ => Err("Invalid arguments for command.".to_owned()),
    }
}

fn run(args: &[String]) -> i32 {
    let action = match parse_args(args) {
        Ok(action) => action,
        Err(error) => {
            eprintln!("Error: {error}");
            eprintln!("Run `forge --help` to see available commands.");
            return 2;
        }
    };

    match action {
        Action::Start => start(),
        Action::Login => cli::login(),
        Action::Projects => cli::projects(),
        Action::Account(action) => cli::account(action),
        Action::Help => {
            print_help();
            0
        }
    }
}

fn print_help() {
    println!("Forge - Git and GitHub in your terminal");
    println!();
    println!("Usage: forge [COMMAND]");
    println!();
    println!("Commands:");
    println!("  login    Authenticate with GitHub using GitHub CLI");
    println!("  account  Manage saved GitHub accounts (list, add, use)");
    println!("  projects List repositories available to your GitHub account");
    println!("  --help   Show this help");
}

fn start() -> i32 {
    match github::gh_auth_status() {
        Ok(()) => match app::run_tui() {
            Ok(()) => 0,
            Err(error) => {
                eprintln!("Error: Could not start the Forge TUI.");
                eprintln!("Cause: {error}");
                eprintln!("Next step: Run Forge in an interactive terminal and try again.");
                1
            }
        },
        Err(cause) => {
            eprintln!("Error: GitHub login is required before Forge can start.");
            eprintln!("Cause: {cause}");
            eprintln!("Next step: Run `forge login`, then start Forge again.");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{AccountAction, Action, parse_args};

    #[test]
    fn parses_login_command_without_startup_gate() {
        assert!(matches!(
            parse_args(&["login".to_owned()]),
            Ok(Action::Login)
        ));
    }

    #[test]
    fn rejects_arguments_to_login() {
        assert!(parse_args(&["login".to_owned(), "--unexpected".to_owned()]).is_err());
    }

    #[test]
    fn parses_projects_command_without_arguments() {
        assert!(matches!(
            parse_args(&["projects".to_owned()]),
            Ok(Action::Projects)
        ));
        assert!(parse_args(&["projects".to_owned(), "--unexpected".to_owned()]).is_err());
    }

    #[test]
    fn parses_start_and_help_actions() {
        assert!(matches!(parse_args(&[]), Ok(Action::Start)));
        assert!(matches!(
            parse_args(&["--help".to_owned()]),
            Ok(Action::Help)
        ));
    }

    #[test]
    fn parses_account_management_commands() {
        assert!(matches!(
            parse_args(&["account".to_owned(), "list".to_owned()]),
            Ok(Action::Account(AccountAction::List))
        ));
        assert!(matches!(
            parse_args(&["account".to_owned(), "add".to_owned()]),
            Ok(Action::Account(AccountAction::Add))
        ));
        assert!(matches!(
            parse_args(&[
                "account".to_owned(),
                "use".to_owned(),
                "octocat".to_owned()
            ]),
            Ok(Action::Account(AccountAction::Use(username))) if username == "octocat"
        ));
        assert!(parse_args(&["account".to_owned()]).is_err());
        assert!(parse_args(&["account".to_owned(), "use".to_owned(), "".to_owned()]).is_err());
    }
}
