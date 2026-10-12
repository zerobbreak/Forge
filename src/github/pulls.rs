//! Pull requests through the GitHub CLI: list, detail, checks, reviews, checkout.
//!
//! Everything here comes from GitHub, so callers label it as remote and show when it
//! was fetched. `gh` finds the repository from the folder's Git remote.

use serde_json::Value;
use std::{io, path::Path, process::Command, process::Stdio};

/// Pull requests requested per list.
pub const LIMIT: usize = 50;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PullFilter {
    Open,
    Merged,
    Closed,
    All,
}

impl PullFilter {
    pub fn next(self) -> Self {
        match self {
            Self::Open => Self::Merged,
            Self::Merged => Self::Closed,
            Self::Closed => Self::All,
            Self::All => Self::Open,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Merged => "merged",
            Self::Closed => "closed",
            Self::All => "all",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PullState {
    Open,
    Closed,
    Merged,
}

/// GitHub's overall review verdict for a pull request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    Approved,
    ChangesRequested,
    ReviewRequired,
    /// No review is required, or none has happened.
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckState {
    Passed,
    Failed,
    Pending,
    Skipped,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Check {
    pub name: String,
    pub workflow: Option<String>,
    pub state: CheckState,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PullRequest {
    pub number: u64,
    pub title: String,
    pub author: String,
    pub state: PullState,
    pub draft: bool,
    pub head: String,
    pub base: String,
    pub updated: u64,
    pub decision: Decision,
    pub checks: Vec<Check>,
    pub additions: usize,
    pub deletions: usize,
    pub changed_files: usize,
    pub labels: Vec<String>,
}

impl PullRequest {
    pub fn count(&self, state: CheckState) -> usize {
        self.checks
            .iter()
            .filter(|check| check.state == state)
            .count()
    }

    /// Checks that ran and counted (skipped ones are left out).
    pub fn counted_checks(&self) -> usize {
        self.checks.len() - self.count(CheckState::Skipped)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReviewState {
    Approved,
    ChangesRequested,
    Commented,
    Dismissed,
    Pending,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Review {
    pub author: String,
    pub state: ReviewState,
    pub submitted: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PullFile {
    pub path: String,
    pub additions: usize,
    pub deletions: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mergeable {
    Clean,
    Conflicting,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PullDetail {
    pub number: u64,
    pub body: String,
    /// The latest review from each reviewer.
    pub reviews: Vec<Review>,
    /// Reviewers asked but not yet heard from.
    pub requested: Vec<String>,
    pub files: Vec<PullFile>,
    pub commits: usize,
    pub comments: usize,
    pub mergeable: Mergeable,
}

/// Runs `gh` in `repo` without prompts, colour or update notices.
fn gh(repo: &Path, arguments: &[&str]) -> Result<String, String> {
    let output = Command::new("gh")
        .args(arguments)
        .current_dir(repo)
        .env("GH_PROMPT_DISABLED", "1")
        .env("GH_NO_UPDATE_NOTIFIER", "1")
        .env("NO_COLOR", "1")
        .stdin(Stdio::null())
        .output()
        .map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                "GitHub CLI (`gh`) is not installed or is not on PATH. Install it from https://cli.github.com, then press r."
                    .to_owned()
            } else {
                format!("Could not run GitHub CLI: {error}")
            }
        })?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
    }
    Err(explain(&String::from_utf8_lossy(&output.stderr)))
}

/// Turns `gh`'s complaint into a cause and a next step.
pub fn explain(stderr: &str) -> String {
    let lower = stderr.to_lowercase();
    let first = stderr
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("GitHub CLI failed without a message.");
    if lower.contains("known github host") || lower.contains("no git remotes") {
        "This repository has no GitHub remote. Add one with: git remote add origin <url>."
            .to_owned()
    } else if lower.contains("gh auth login")
        || lower.contains("not logged in")
        || lower.contains("http 401")
        || lower.contains("bad credentials")
    {
        "You are not signed in to GitHub. Run: forge login.".to_owned()
    } else if lower.contains("could not resolve host")
        || lower.contains("error connecting")
        || lower.contains("timeout")
        || lower.contains("dial tcp")
    {
        "Could not reach GitHub. Check your connection, then press r.".to_owned()
    } else if lower.contains("could not resolve to a repository") || lower.contains("http 404") {
        "GitHub cannot find this repository, or your account cannot see it. Check the remote and the active account (@).".to_owned()
    } else if lower.contains("rate limit") {
        "GitHub rate limit reached. Wait a few minutes, then press r.".to_owned()
    } else {
        first.to_owned()
    }
}

const LIST_FIELDS: &str = "number,title,author,state,isDraft,headRefName,baseRefName,updatedAt,reviewDecision,statusCheckRollup,additions,deletions,changedFiles,labels";
const DETAIL_FIELDS: &str = "body,latestReviews,reviewRequests,files,commits,comments,mergeable";

pub fn list_pulls(repo: &Path, filter: PullFilter) -> Result<Vec<PullRequest>, String> {
    let limit = LIMIT.to_string();
    let output = gh(
        repo,
        &[
            "pr",
            "list",
            "--state",
            filter.label(),
            "--limit",
            &limit,
            "--json",
            LIST_FIELDS,
        ],
    )?;
    parse_list(&output)
}

pub fn read_pull(repo: &Path, number: u64) -> Result<PullDetail, String> {
    let number_text = number.to_string();
    let output = gh(repo, &["pr", "view", &number_text, "--json", DETAIL_FIELDS])?;
    parse_detail(number, &output)
}

/// Checks out the pull request's branch (fetching it when needed).
pub fn checkout_pull(repo: &Path, number: u64) -> Result<String, String> {
    gh(repo, &["pr", "checkout", &number.to_string()])?;
    Ok(format!("Checked out pull request #{number}."))
}

/// Opens the pull request in the default browser.
pub fn open_pull(repo: &Path, number: u64) -> Result<String, String> {
    gh(repo, &["pr", "view", &number.to_string(), "--web"])?;
    Ok(format!("Opened #{number} in your browser."))
}

fn text(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn count(value: &Value, key: &str) -> usize {
    value
        .get(key)
        .and_then(Value::as_u64)
        .map_or(0, |count| count as usize)
}

fn login(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(|author| author.get("login"))
        .and_then(Value::as_str)
        .unwrap_or("ghost")
        .to_owned()
}

pub fn parse_list(json: &str) -> Result<Vec<PullRequest>, String> {
    let value: Value = serde_json::from_str(json)
        .map_err(|error| format!("GitHub CLI returned something unreadable ({error})."))?;
    let items = value
        .as_array()
        .ok_or("GitHub CLI returned something unexpected instead of a list.")?;
    Ok(items.iter().map(parse_pull).collect())
}

fn parse_pull(item: &Value) -> PullRequest {
    let state = match item.get("state").and_then(Value::as_str) {
        Some("MERGED") => PullState::Merged,
        Some("CLOSED") => PullState::Closed,
        _ => PullState::Open,
    };
    let decision = match item.get("reviewDecision").and_then(Value::as_str) {
        Some("APPROVED") => Decision::Approved,
        Some("CHANGES_REQUESTED") => Decision::ChangesRequested,
        Some("REVIEW_REQUIRED") => Decision::ReviewRequired,
        _ => Decision::None,
    };
    let checks = item
        .get("statusCheckRollup")
        .and_then(Value::as_array)
        .map(|checks| checks.iter().map(parse_check).collect())
        .unwrap_or_default();
    let labels = item
        .get("labels")
        .and_then(Value::as_array)
        .map(|labels| {
            labels
                .iter()
                .filter_map(|label| label.get("name").and_then(Value::as_str))
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    PullRequest {
        number: item.get("number").and_then(Value::as_u64).unwrap_or(0),
        title: text(item, "title"),
        author: login(item, "author"),
        state,
        draft: item
            .get("isDraft")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        head: text(item, "headRefName"),
        base: text(item, "baseRefName"),
        updated: parse_time(&text(item, "updatedAt")).unwrap_or(0),
        decision,
        checks,
        additions: count(item, "additions"),
        deletions: count(item, "deletions"),
        changed_files: count(item, "changedFiles"),
        labels,
    }
}

/// A `CheckRun` (GitHub Actions and apps) or a `StatusContext` (commit statuses).
fn parse_check(item: &Value) -> Check {
    let state = if item.get("__typename").and_then(Value::as_str) == Some("StatusContext") {
        match item.get("state").and_then(Value::as_str) {
            Some("SUCCESS") => CheckState::Passed,
            Some("PENDING" | "EXPECTED") => CheckState::Pending,
            _ => CheckState::Failed,
        }
    } else if item.get("status").and_then(Value::as_str) != Some("COMPLETED") {
        CheckState::Pending
    } else {
        match item.get("conclusion").and_then(Value::as_str) {
            Some("SUCCESS") => CheckState::Passed,
            Some("SKIPPED" | "NEUTRAL") => CheckState::Skipped,
            _ => CheckState::Failed,
        }
    };
    let name = item
        .get("name")
        .or_else(|| item.get("context"))
        .and_then(Value::as_str)
        .unwrap_or("check")
        .to_owned();
    Check {
        name,
        workflow: item
            .get("workflowName")
            .and_then(Value::as_str)
            .filter(|workflow| !workflow.is_empty())
            .map(str::to_owned),
        state,
    }
}

pub fn parse_detail(number: u64, json: &str) -> Result<PullDetail, String> {
    let value: Value = serde_json::from_str(json)
        .map_err(|error| format!("GitHub CLI returned something unreadable ({error})."))?;
    let reviews = value
        .get("latestReviews")
        .and_then(Value::as_array)
        .map(|reviews| {
            reviews
                .iter()
                .map(|review| Review {
                    author: login(review, "author"),
                    state: match review.get("state").and_then(Value::as_str) {
                        Some("APPROVED") => ReviewState::Approved,
                        Some("CHANGES_REQUESTED") => ReviewState::ChangesRequested,
                        Some("DISMISSED") => ReviewState::Dismissed,
                        Some("PENDING") => ReviewState::Pending,
                        _ => ReviewState::Commented,
                    },
                    submitted: parse_time(&text(review, "submittedAt")).unwrap_or(0),
                })
                .collect()
        })
        .unwrap_or_default();
    let requested = value
        .get("reviewRequests")
        .and_then(Value::as_array)
        .map(|requests| {
            requests
                .iter()
                // Users have a `login`; teams have a `name`.
                .filter_map(|request| {
                    request
                        .get("login")
                        .or_else(|| request.get("name"))
                        .and_then(Value::as_str)
                })
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    let files = value
        .get("files")
        .and_then(Value::as_array)
        .map(|files| {
            files
                .iter()
                .map(|file| PullFile {
                    path: text(file, "path"),
                    additions: count(file, "additions"),
                    deletions: count(file, "deletions"),
                })
                .collect()
        })
        .unwrap_or_default();
    let length = |key: &str| value.get(key).and_then(Value::as_array).map_or(0, Vec::len);
    Ok(PullDetail {
        number,
        body: text(&value, "body").replace("\r\n", "\n"),
        reviews,
        requested,
        files,
        commits: length("commits"),
        comments: length("comments"),
        mergeable: match value.get("mergeable").and_then(Value::as_str) {
            Some("MERGEABLE") => Mergeable::Clean,
            Some("CONFLICTING") => Mergeable::Conflicting,
            _ => Mergeable::Unknown,
        },
    })
}

/// `2026-10-11T23:03:09Z` to Unix seconds.
pub fn parse_time(iso: &str) -> Option<u64> {
    let (date, time) = iso.trim_end_matches('Z').split_once('T')?;
    let mut date = date.split('-').map(|part| part.parse::<i64>());
    let (year, month, day) = (date.next()?.ok()?, date.next()?.ok()?, date.next()?.ok()?);
    let mut time = time.split(':').map(|part| part.parse::<i64>());
    let (hour, minute, second) = (time.next()?.ok()?, time.next()?.ok()?, time.next()?.ok()?);
    // Civil date to days since 1970-01-01 (Howard Hinnant's algorithm).
    let year = year - i64::from(month <= 2);
    let era = year.div_euclid(400);
    let year_of_era = year.rem_euclid(400);
    let month_index = (month + 9) % 12;
    let day_of_year = (153 * month_index + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + day_of_era - 719_468;
    u64::try_from(days * 86_400 + hour * 3_600 + minute * 60 + second).ok()
}

#[cfg(test)]
mod tests {
    use super::{
        CheckState, Decision, Mergeable, PullFilter, PullState, ReviewState, explain, parse_detail,
        parse_list, parse_time,
    };
    use crate::ui::format_utc;

    /// Shaped like `gh pr list --json …` output from real repositories.
    const LIST: &str = r#"[
      {"additions":545,"author":{"id":"x","is_bot":false,"login":"BagToad","name":"Kynan"},
       "baseRefName":"trunk","changedFiles":30,"deletions":280,"headRefName":"feat/artifact",
       "isDraft":false,"labels":[{"id":"1","name":"external","description":"","color":"2700AF"}],
       "number":14640,"reviewDecision":"REVIEW_REQUIRED","state":"OPEN",
       "statusCheckRollup":[
         {"__typename":"CheckRun","conclusion":"SUCCESS","name":"build","status":"COMPLETED","workflowName":"CI"},
         {"__typename":"CheckRun","conclusion":"SKIPPED","name":"label","status":"COMPLETED","workflowName":"Triage"},
         {"__typename":"CheckRun","conclusion":"","name":"lint","status":"IN_PROGRESS","workflowName":"CI"},
         {"__typename":"CheckRun","conclusion":"FAILURE","name":"test","status":"COMPLETED","workflowName":"CI"},
         {"__typename":"StatusContext","context":"ci/legacy","state":"PENDING","targetUrl":"u"},
         {"__typename":"StatusContext","context":"ci/other","state":"SUCCESS","targetUrl":"u"}],
       "title":"Address review feedback","updatedAt":"2026-10-09T20:04:39Z"},
      {"additions":1,"author":{"login":"app/dependabot","is_bot":true},"baseRefName":"main","changedFiles":1,
       "deletions":1,"headRefName":"dependabot/x","isDraft":true,"labels":[],"number":7,
       "reviewDecision":"","state":"MERGED","statusCheckRollup":[],"title":"Bump x","updatedAt":"2026-10-01T00:00:00Z"}
    ]"#;

    const DETAIL: &str = r#"{
      "body":"Fixes the cache.\r\nSecond line.",
      "latestReviews":[
        {"author":{"login":"williammartin"},"state":"APPROVED","submittedAt":"2026-10-09T14:33:25Z"},
        {"author":{"login":"hubot"},"state":"CHANGES_REQUESTED","submittedAt":"2026-10-10T09:00:00Z"},
        {"author":{"login":"octocat"},"state":"COMMENTED","submittedAt":"2026-10-10T10:00:00Z"}],
      "reviewRequests":[{"__typename":"User","login":"tidy-dev"},{"__typename":"Team","name":"core","slug":"core"}],
      "files":[{"path":"go.mod","additions":1,"deletions":1,"changeType":"MODIFIED"},
               {"path":"go.sum","additions":2,"deletions":2,"changeType":"MODIFIED"}],
      "commits":[{"oid":"a"},{"oid":"b"},{"oid":"c"}],
      "comments":[{"body":"hi"}],
      "mergeable":"CONFLICTING"
    }"#;

    #[test]
    fn parses_a_pull_request_list() {
        let pulls = parse_list(LIST).expect("list parses");
        assert_eq!(pulls.len(), 2);
        let first = &pulls[0];
        assert_eq!(
            (first.number, first.title.as_str()),
            (14640, "Address review feedback")
        );
        assert_eq!(first.author, "BagToad");
        assert_eq!(
            (first.head.as_str(), first.base.as_str()),
            ("feat/artifact", "trunk")
        );
        assert_eq!(first.state, PullState::Open);
        assert_eq!(first.decision, Decision::ReviewRequired);
        assert_eq!(
            (first.additions, first.deletions, first.changed_files),
            (545, 280, 30)
        );
        assert_eq!(first.labels, vec!["external".to_owned()]);
        assert_eq!(first.updated, parse_time("2026-10-09T20:04:39Z").unwrap());
        assert!(!first.draft);

        let second = &pulls[1];
        assert_eq!(second.state, PullState::Merged);
        assert!(second.draft);
        assert_eq!(second.decision, Decision::None);
        assert!(second.checks.is_empty());
        assert_eq!(second.author, "app/dependabot");
    }

    #[test]
    fn classifies_check_runs_and_commit_statuses() {
        let first = &parse_list(LIST).unwrap()[0];
        assert_eq!(first.checks.len(), 6);
        assert_eq!(first.count(CheckState::Passed), 2);
        assert_eq!(first.count(CheckState::Skipped), 1);
        assert_eq!(
            first.count(CheckState::Pending),
            2,
            "a running job and a pending status"
        );
        assert_eq!(first.count(CheckState::Failed), 1);
        assert_eq!(first.counted_checks(), 5, "skipped checks do not count");
        let lint = first.checks.iter().find(|c| c.name == "lint").unwrap();
        assert_eq!(lint.workflow.as_deref(), Some("CI"));
        let legacy = first.checks.iter().find(|c| c.name == "ci/legacy").unwrap();
        assert_eq!(legacy.workflow, None);
    }

    #[test]
    fn parses_pull_request_detail() {
        let detail = parse_detail(14640, DETAIL).expect("detail parses");
        assert_eq!(detail.body, "Fixes the cache.\nSecond line.");
        assert_eq!(detail.reviews.len(), 3);
        assert_eq!(detail.reviews[0].author, "williammartin");
        assert_eq!(detail.reviews[0].state, ReviewState::Approved);
        assert_eq!(detail.reviews[1].state, ReviewState::ChangesRequested);
        assert_eq!(detail.reviews[2].state, ReviewState::Commented);
        assert_eq!(
            detail.requested,
            vec!["tidy-dev".to_owned(), "core".to_owned()]
        );
        assert_eq!(detail.files.len(), 2);
        assert_eq!(
            (detail.files[1].path.as_str(), detail.files[1].additions),
            ("go.sum", 2)
        );
        assert_eq!((detail.commits, detail.comments), (3, 1));
        assert_eq!(detail.mergeable, Mergeable::Conflicting);
    }

    #[test]
    fn missing_fields_degrade_instead_of_failing() {
        let detail = parse_detail(1, "{}").expect("empty object is fine");
        assert!(detail.files.is_empty() && detail.reviews.is_empty());
        assert_eq!(detail.mergeable, Mergeable::Unknown);
        assert!(parse_list("[]").unwrap().is_empty());
    }

    #[test]
    fn rejects_output_that_is_not_json() {
        assert!(parse_list("not json").unwrap_err().contains("unreadable"));
        assert!(parse_list("{}").unwrap_err().contains("unexpected"));
        assert!(parse_detail(1, "<html>").is_err());
    }

    #[test]
    fn iso_times_round_trip_with_the_date_formatter() {
        for iso in [
            "1970-01-01T00:00:00Z",
            "2000-02-29T00:00:00Z",
            "2026-10-11T23:03:00Z",
        ] {
            let seconds = parse_time(iso).expect("parses");
            assert_eq!(
                format_utc(seconds),
                format!("{} {} UTC", &iso[..10], &iso[11..16]),
                "{iso}"
            );
        }
        assert_eq!(parse_time("garbage"), None);
        assert_eq!(parse_time("2026-10-11"), None);
    }

    #[test]
    fn explains_gh_failures_with_a_next_step() {
        assert!(
            explain("none of the git remotes configured for this repository point to a known GitHub host.")
                .contains("git remote add")
        );
        assert!(
            explain("To get started with GitHub CLI, please run:  gh auth login")
                .contains("forge login")
        );
        assert!(explain("error connecting to api.github.com").contains("Could not reach GitHub"));
        assert!(
            explain("GraphQL: Could not resolve to a Repository with the name 'x/y'.")
                .contains("cannot find")
        );
        assert!(explain("API rate limit exceeded").contains("rate limit"));
        assert_eq!(explain("something odd\nsecond line"), "something odd");
        assert!(explain("").contains("without a message"));
    }

    #[test]
    fn the_filter_cycles_through_every_state() {
        let mut filter = PullFilter::Open;
        let mut seen = Vec::new();
        for _ in 0..4 {
            seen.push(filter.label());
            filter = filter.next();
        }
        assert_eq!(seen, ["open", "merged", "closed", "all"]);
        assert_eq!(filter, PullFilter::Open);
    }

    /// Run with `cargo test -- --ignored` while signed in to `gh` and online.
    #[test]
    #[ignore = "needs network access and a signed-in GitHub CLI"]
    fn reads_real_pull_requests_through_gh() {
        use super::{list_pulls, read_pull};
        use crate::test_support::{TestDirectory, run_git};
        let directory = TestDirectory::new("pulls-live");
        let repo = directory.path();
        run_git(repo, &["init", "-b", "main"]);
        run_git(
            repo,
            &["remote", "add", "origin", "https://github.com/cli/cli.git"],
        );

        let pulls = list_pulls(repo, PullFilter::All).expect("gh lists pull requests");
        assert!(!pulls.is_empty());
        assert!(
            pulls
                .iter()
                .all(|pull| pull.number > 0 && !pull.title.is_empty())
        );
        assert!(pulls.iter().all(|pull| pull.updated > 0));
        let detail = read_pull(repo, pulls[0].number).expect("gh reads one pull request");
        assert_eq!(detail.number, pulls[0].number);

        let lonely = TestDirectory::new("pulls-no-remote");
        run_git(lonely.path(), &["init", "-b", "main"]);
        let error = list_pulls(lonely.path(), PullFilter::Open).unwrap_err();
        assert!(error.contains("no GitHub remote"), "{error}");
    }
}
