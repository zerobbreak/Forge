//! Find in files and replace: `git grep` finds candidate lines, a Rust regex marks the
//! exact matches, and replace edits only the lines the user left selected, after a
//! recovery snapshot of each file (ADR 0002).

use super::{
    changes::snapshot,
    repository::{GitOptions, run_git},
};
use regex::{NoExpand, Regex, RegexBuilder};
use std::{
    collections::BTreeMap,
    fs,
    ops::Range,
    path::{Component, Path},
};

/// Matches kept from one search; the rest are counted as "more".
pub const MAX_MATCHES: usize = 1000;
/// Files larger than this are not read for context or replaced.
const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;
/// Longest line kept; longer ones are cut so a minified file cannot flood the screen.
const MAX_LINE_CHARS: usize = 400;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct SearchOptions {
    pub case_sensitive: bool,
    pub regex: bool,
    pub whole_word: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Match {
    pub path: String,
    /// 1-based.
    pub line: usize,
    /// The line as found, without its line ending.
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchResult {
    pub matches: Vec<Match>,
    /// More than [`MAX_MATCHES`] lines matched.
    pub truncated: bool,
}

/// Builds the matcher for `query`: literal text unless `options.regex` is set.
pub fn build_matcher(query: &str, options: &SearchOptions) -> Result<Regex, String> {
    if query.is_empty() {
        return Err("Type something to look for.".to_owned());
    }
    let body = if options.regex {
        query.to_owned()
    } else {
        regex::escape(query)
    };
    let pattern = if options.whole_word {
        format!(r"\b(?:{body})\b")
    } else {
        body
    };
    RegexBuilder::new(&pattern)
        .case_insensitive(!options.case_sensitive)
        .size_limit(1 << 20)
        .build()
        .map_err(|error| {
            // The first line is the useful one; the rest repeats the pattern with a caret.
            let text = error.to_string();
            let reason = text
                .lines()
                .rev()
                .find(|line| line.trim_start().starts_with("error:"))
                .or_else(|| text.lines().next())
                .unwrap_or("invalid pattern");
            format!(
                "That is not a valid pattern ({}).",
                reason.trim().trim_start_matches("error:").trim()
            )
        })
}

/// Byte ranges of every match in `text`.
pub fn find_spans(matcher: &Regex, text: &str) -> Vec<Range<usize>> {
    matcher
        .find_iter(text)
        .filter(|found| !found.range().is_empty())
        .map(|found| found.range())
        .collect()
}

/// Searches tracked and untracked (but not ignored) text files in `repo`.
pub fn search(repo: &Path, query: &str, options: &SearchOptions) -> Result<SearchResult, String> {
    // Validate with the same matcher the replace step will use.
    build_matcher(query, options)?;
    let mut arguments = vec![
        "grep",
        "-n",
        "-z",
        "-I",
        "--no-color",
        "--untracked",
        "--exclude-standard",
    ];
    if !options.case_sensitive {
        arguments.push("-i");
    }
    if options.whole_word {
        arguments.push("-w");
    }
    arguments.push(if options.regex { "-E" } else { "-F" });
    arguments.extend(["-e", query, "--"]);
    let output = run_git(
        repo,
        &arguments,
        GitOptions {
            // Exit code 1 means "nothing found".
            allowed_codes: &[1],
            ..GitOptions::default()
        },
    )
    .map_err(|detail| {
        // `git grep` prints "fatal: command line, 'x': Unmatched ( or \(" for bad patterns.
        let reason = detail.rsplit(": ").next().unwrap_or(&detail).trim();
        format!("Git could not search for that ({reason}).")
    })?;
    Ok(parse_grep(&String::from_utf8_lossy(&output)))
}

/// Parses `git grep -n -z` output: `path NUL line NUL text` per line.
pub fn parse_grep(output: &str) -> SearchResult {
    let mut matches = Vec::new();
    let mut truncated = false;
    for record in output.split('\n') {
        let mut fields = record.splitn(3, '\0');
        let (Some(path), Some(line), Some(text)) = (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        let Ok(line) = line.parse::<usize>() else {
            continue;
        };
        if matches.len() == MAX_MATCHES {
            truncated = true;
            break;
        }
        let text = text.trim_end_matches('\r');
        let text: String = if text.chars().count() > MAX_LINE_CHARS {
            text.chars().take(MAX_LINE_CHARS).collect()
        } else {
            text.to_owned()
        };
        matches.push(Match {
            path: path.to_owned(),
            line,
            text,
        });
    }
    SearchResult { matches, truncated }
}

/// Lines around a match, for the preview.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Context {
    /// Line number of `lines[0]`, 1-based.
    pub first_line: usize,
    pub lines: Vec<String>,
}

pub fn read_context(
    repo: &Path,
    path: &str,
    line: usize,
    radius: usize,
) -> Result<Context, String> {
    let full = repo.join(checked_path(path)?);
    let size = fs::metadata(&full)
        .map_err(|error| format!("Could not open {path}: {error}."))?
        .len();
    if size > MAX_FILE_BYTES {
        return Err(format!("{path} is too large to preview."));
    }
    let bytes = fs::read(&full).map_err(|error| format!("Could not read {path}: {error}."))?;
    let text = String::from_utf8_lossy(&bytes);
    let first = line.saturating_sub(radius).max(1);
    let lines: Vec<String> = text
        .split('\n')
        .skip(first - 1)
        .take(line + radius - first + 1)
        .map(|line| {
            let line = line.trim_end_matches('\r');
            line.chars().take(MAX_LINE_CHARS).collect()
        })
        .collect();
    Ok(Context {
        first_line: first,
        lines,
    })
}

/// `text` with every match replaced; in regex mode `$1` and `${name}` refer to groups.
pub fn replace_in_line(
    matcher: &Regex,
    text: &str,
    replacement: &str,
    options: &SearchOptions,
) -> String {
    if options.regex {
        matcher.replace_all(text, replacement).into_owned()
    } else {
        matcher
            .replace_all(text, NoExpand(replacement))
            .into_owned()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReplaceReport {
    pub lines: usize,
    pub files: usize,
    /// Lines whose text no longer matched what the search saw, so they were left alone.
    pub changed_since_search: usize,
    /// Files that could not be edited safely (not UTF-8, too large, gone).
    pub skipped_files: Vec<String>,
    /// `(path, snapshot ref)` for each file that was edited.
    pub snapshots: Vec<(String, String)>,
}

/// Replaces matches on the chosen lines. Each file is read, edited in memory and
/// written back only after a recovery snapshot of it exists. A line is edited only
/// if its text is still exactly what the search found.
pub fn replace_matches(
    repo: &Path,
    query: &str,
    replacement: &str,
    options: &SearchOptions,
    targets: &[Match],
) -> Result<ReplaceReport, String> {
    let matcher = build_matcher(query, options)?;
    let mut by_file: BTreeMap<&str, Vec<&Match>> = BTreeMap::new();
    for target in targets {
        by_file.entry(&target.path).or_default().push(target);
    }
    let mut report = ReplaceReport::default();
    for (path, wanted) in by_file {
        let full = repo.join(checked_path(path)?);
        let usable =
            fs::metadata(&full).is_ok_and(|meta| meta.is_file() && meta.len() <= MAX_FILE_BYTES);
        let content = usable
            .then(|| fs::read(&full).ok())
            .flatten()
            .and_then(|bytes| String::from_utf8(bytes).ok());
        let Some(content) = content else {
            report.skipped_files.push(path.to_owned());
            continue;
        };

        let mut lines: Vec<String> = content.split_inclusive('\n').map(str::to_owned).collect();
        let mut edited = 0;
        for target in wanted {
            let Some(raw) = lines.get_mut(target.line.wrapping_sub(1)) else {
                report.changed_since_search += 1;
                continue;
            };
            let ending_len = if raw.ends_with("\r\n") {
                2
            } else {
                usize::from(raw.ends_with('\n'))
            };
            let (body, ending) = raw.split_at(raw.len() - ending_len);
            // The search shows a cut-down line, so compare on what it could see.
            let seen: String = body.chars().take(MAX_LINE_CHARS).collect();
            if seen != target.text {
                report.changed_since_search += 1;
                continue;
            }
            let new_body = replace_in_line(&matcher, body, replacement, options);
            if new_body != body {
                *raw = format!("{new_body}{ending}");
                edited += 1;
            }
        }
        if edited == 0 {
            continue;
        }
        let saved = snapshot(repo, path, "replace")?;
        fs::write(&full, lines.concat()).map_err(|error| {
            format!(
                "Could not write {path}: {error}. Your original is saved as {saved}; restore it with: git checkout {saved} -- {path}"
            )
        })?;
        report.lines += edited;
        report.files += 1;
        report.snapshots.push((path.to_owned(), saved));
    }
    Ok(report)
}

fn checked_path(path: &str) -> Result<&str, String> {
    let escapes = Path::new(path)
        .components()
        .any(|part| !matches!(part, Component::Normal(_) | Component::CurDir));
    if path.is_empty() || escapes {
        Err(format!("Refusing to touch unexpected path {path:?}."))
    } else {
        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_MATCHES, Match, SearchOptions, build_matcher, find_spans, parse_grep, read_context,
        replace_in_line, replace_matches, search,
    };
    use crate::test_support::{TestDirectory, git_text, run_git};
    use std::{
        fs,
        path::{Path, PathBuf},
    };

    fn repository(directory: &TestDirectory) -> PathBuf {
        let repo = directory.path().join("repo");
        fs::create_dir_all(&repo).unwrap();
        run_git(&repo, &["init", "-b", "main"]);
        run_git(&repo, &["config", "user.name", "Forge Test"]);
        run_git(&repo, &["config", "user.email", "forge@example.test"]);
        run_git(&repo, &["config", "commit.gpgsign", "false"]);
        run_git(&repo, &["config", "core.autocrlf", "false"]);
        repo
    }

    fn write(repo: &Path, path: &str, text: &str) {
        let full = repo.join(path);
        fs::create_dir_all(full.parent().unwrap()).unwrap();
        fs::write(full, text).unwrap();
    }

    fn commit_all(repo: &Path) {
        run_git(repo, &["add", "."]);
        run_git(repo, &["commit", "-m", "files"]);
    }

    fn read(repo: &Path, path: &str) -> String {
        fs::read_to_string(repo.join(path)).unwrap()
    }

    fn found(result: &super::SearchResult) -> Vec<(String, usize)> {
        result
            .matches
            .iter()
            .map(|m| (m.path.clone(), m.line))
            .collect()
    }

    #[test]
    fn builds_literal_regex_word_and_case_matchers() {
        let literal = build_matcher("a.b", &SearchOptions::default()).unwrap();
        assert_eq!(find_spans(&literal, "a.b axb A.B"), vec![0..3, 8..11]);
        let sensitive = SearchOptions {
            case_sensitive: true,
            ..Default::default()
        };
        assert_eq!(
            find_spans(&build_matcher("a.b", &sensitive).unwrap(), "a.b A.B"),
            vec![0..3]
        );
        let regex = SearchOptions {
            regex: true,
            ..Default::default()
        };
        assert_eq!(
            find_spans(&build_matcher("a.b", &regex).unwrap(), "a.b axb"),
            vec![0..3, 4..7]
        );
        let word = SearchOptions {
            whole_word: true,
            ..Default::default()
        };
        assert_eq!(
            find_spans(&build_matcher("cat", &word).unwrap(), "cat concat cat."),
            vec![0..3, 11..14]
        );
        // A pattern that can match nothing never produces empty spans.
        assert!(find_spans(&build_matcher("x*", &regex).unwrap(), "abc").is_empty());
    }

    #[test]
    fn reports_bad_patterns_and_empty_queries_in_plain_words() {
        let regex = SearchOptions {
            regex: true,
            ..Default::default()
        };
        let error = build_matcher("(unclosed", &regex).unwrap_err();
        assert!(
            error.starts_with("That is not a valid pattern ("),
            "{error}"
        );
        assert!(error.len() < 160, "{error}");
        assert_eq!(
            build_matcher("", &regex).unwrap_err(),
            "Type something to look for."
        );
        // Literal mode never fails on punctuation.
        assert!(build_matcher("(unclosed", &SearchOptions::default()).is_ok());
    }

    #[test]
    fn parses_grep_records_and_caps_the_result() {
        let output =
            "src/a.rs\u{0}12\u{0}let x = 1;\r\nsrc/b rs.txt\u{0}3\u{0}tab\there\n\nnoise\n";
        let result = parse_grep(output);
        assert_eq!(result.matches.len(), 2);
        assert_eq!(
            result.matches[0].text, "let x = 1;",
            "line endings are dropped"
        );
        assert_eq!(result.matches[1].path, "src/b rs.txt");
        assert!(!result.truncated);

        let many: String = (1..=MAX_MATCHES + 5)
            .map(|n| format!("f\u{0}{n}\u{0}x\n"))
            .collect();
        let result = parse_grep(&many);
        assert_eq!(result.matches.len(), MAX_MATCHES);
        assert!(result.truncated);
        let long = format!("f\u{0}1\u{0}{}\n", "y".repeat(5000));
        assert_eq!(parse_grep(&long).matches[0].text.chars().count(), 400);
    }

    #[test]
    fn finds_lines_in_tracked_and_untracked_files_but_not_ignored_or_binary_ones() {
        let directory = TestDirectory::new("search-find");
        let repo = repository(&directory);
        write(
            &repo,
            "src/a.rs",
            "fn main() {\n    let Needle = 1;\n}\n// needle again\n",
        );
        write(&repo, "notes/b.txt", "nothing here\n");
        write(&repo, ".gitignore", "ignored/\n");
        commit_all(&repo);
        write(&repo, "untracked.txt", "a needle in a new file\n");
        write(&repo, "ignored/c.txt", "needle in an ignored file\n");
        fs::write(repo.join("blob.bin"), b"needle\0binary").unwrap();

        let result = search(&repo, "needle", &SearchOptions::default()).expect("searches");
        assert_eq!(
            found(&result),
            vec![
                ("src/a.rs".to_owned(), 2),
                ("src/a.rs".to_owned(), 4),
                ("untracked.txt".to_owned(), 1),
            ]
        );
        assert_eq!(result.matches[0].text, "    let Needle = 1;");

        let sensitive = SearchOptions {
            case_sensitive: true,
            ..Default::default()
        };
        assert_eq!(
            found(&search(&repo, "needle", &sensitive).unwrap()).len(),
            2
        );
        let none = search(&repo, "absent-word", &SearchOptions::default())
            .expect("no match is not an error");
        assert!(none.matches.is_empty());
    }

    #[test]
    fn regex_and_whole_word_searches_and_bad_regexes() {
        let directory = TestDirectory::new("search-regex");
        let repo = repository(&directory);
        write(&repo, "a.txt", "cat\nconcat\nbobcat cat\nc4t\n");
        commit_all(&repo);
        let word = SearchOptions {
            whole_word: true,
            ..Default::default()
        };
        assert_eq!(
            found(&search(&repo, "cat", &word).unwrap())
                .iter()
                .map(|f| f.1)
                .collect::<Vec<_>>(),
            [1, 3]
        );
        let regex = SearchOptions {
            regex: true,
            ..Default::default()
        };
        assert_eq!(found(&search(&repo, "c[a4]t$", &regex).unwrap()).len(), 4);
        let error = search(&repo, "(oops", &regex).unwrap_err();
        assert!(error.starts_with("That is not a valid pattern"), "{error}");
    }

    #[test]
    fn a_query_that_looks_like_an_option_is_just_text() {
        let directory = TestDirectory::new("search-option");
        let repo = repository(&directory);
        write(&repo, "a.txt", "run with --force please\n");
        commit_all(&repo);
        let result =
            search(&repo, "--force", &SearchOptions::default()).expect("not parsed as a flag");
        assert_eq!(found(&result), vec![("a.txt".to_owned(), 1)]);
    }

    #[test]
    fn reads_context_around_a_line() {
        let directory = TestDirectory::new("search-context");
        let repo = repository(&directory);
        let text: String = (1..=10).map(|n| format!("line {n}\r\n")).collect();
        write(&repo, "a.txt", &text);
        let context = read_context(&repo, "a.txt", 5, 2).unwrap();
        assert_eq!(context.first_line, 3);
        assert_eq!(
            context.lines,
            ["line 3", "line 4", "line 5", "line 6", "line 7"]
        );
        let top = read_context(&repo, "a.txt", 1, 3).unwrap();
        assert_eq!((top.first_line, top.lines.len()), (1, 4));
        assert!(read_context(&repo, "../outside.txt", 1, 1).is_err());
        assert!(read_context(&repo, "missing.txt", 1, 1).is_err());
    }

    #[test]
    fn replaces_literal_text_without_expanding_dollar_signs() {
        let literal = build_matcher("price", &SearchOptions::default()).unwrap();
        assert_eq!(
            replace_in_line(
                &literal,
                "the Price and price",
                "$1.00",
                &SearchOptions::default()
            ),
            "the $1.00 and $1.00"
        );
        let regex = SearchOptions {
            regex: true,
            ..Default::default()
        };
        let matcher = build_matcher(r"(\w+)@(\w+)", &regex).unwrap();
        assert_eq!(
            replace_in_line(&matcher, "me@host", "$2 at $1", &regex),
            "host at me"
        );
    }

    fn targets(repo: &Path, query: &str, options: &SearchOptions) -> Vec<Match> {
        search(repo, query, options).unwrap().matches
    }

    #[test]
    fn replaces_selected_lines_snapshots_each_file_and_keeps_line_endings() {
        let directory = TestDirectory::new("search-replace");
        let repo = repository(&directory);
        write(&repo, "unix.txt", "old name\nkeep\nold again\n");
        write(&repo, "windows.txt", "old name\r\nkeep\r\n");
        write(&repo, "last.txt", "no newline: old");
        commit_all(&repo);
        let options = SearchOptions::default();
        let all = targets(&repo, "old", &options);
        assert_eq!(all.len(), 4);

        // Leave the second match in unix.txt (line 3) out.
        let chosen: Vec<Match> = all
            .iter()
            .filter(|m| !(m.path == "unix.txt" && m.line == 3))
            .cloned()
            .collect();
        let report = replace_matches(&repo, "old", "new", &options, &chosen).expect("replaces");
        assert_eq!((report.lines, report.files), (3, 3));
        assert_eq!(report.changed_since_search, 0);
        assert_eq!(read(&repo, "unix.txt"), "new name\nkeep\nold again\n");
        assert_eq!(read(&repo, "windows.txt"), "new name\r\nkeep\r\n");
        assert_eq!(read(&repo, "last.txt"), "no newline: new");

        assert_eq!(report.snapshots.len(), 3);
        let refs: std::collections::HashSet<_> = report.snapshots.iter().map(|s| &s.1).collect();
        assert_eq!(refs.len(), 3, "every file has its own snapshot");
        for (path, saved) in &report.snapshots {
            assert!(saved.starts_with("refs/forge/snapshots/"));
            // The snapshot holds the file as it was, so the undo command really works.
            let original = git_text(&repo, &["show", &format!("{saved}:{path}")]);
            assert!(original.contains("old"), "{path}");
        }
        let (path, saved) = &report.snapshots[0];
        run_git(&repo, &["checkout", saved, "--", path]);
        assert!(read(&repo, path).contains("old"));
    }

    #[test]
    fn replace_skips_lines_that_changed_since_the_search_and_never_guesses() {
        let directory = TestDirectory::new("search-stale");
        let repo = repository(&directory);
        write(&repo, "a.txt", "old one\nold two\n");
        commit_all(&repo);
        let options = SearchOptions::default();
        let found = targets(&repo, "old", &options);
        // The file is edited after the search: line 1 changes, line 2 stays.
        write(&repo, "a.txt", "something else entirely\nold two\n");
        let report = replace_matches(&repo, "old", "new", &options, &found).expect("replaces");
        assert_eq!((report.lines, report.changed_since_search), (1, 1));
        assert_eq!(read(&repo, "a.txt"), "something else entirely\nnew two\n");

        // A line number past the end of a shorter file is also left alone.
        write(&repo, "a.txt", "short\n");
        let report = replace_matches(&repo, "old", "new", &options, &found).unwrap();
        assert_eq!(
            (report.lines, report.changed_since_search, report.files),
            (0, 2, 0)
        );
        assert_eq!(read(&repo, "a.txt"), "short\n");
        assert!(
            report.snapshots.is_empty(),
            "nothing edited, nothing to snapshot"
        );
    }

    #[test]
    fn replace_supports_capture_groups_and_whole_words() {
        let directory = TestDirectory::new("search-groups");
        let repo = repository(&directory);
        write(&repo, "a.txt", "fn old_name(a: i32)\nconcat cat cat\n");
        commit_all(&repo);
        let regex = SearchOptions {
            regex: true,
            case_sensitive: true,
            ..Default::default()
        };
        let found = targets(&repo, r"fn (\w+)\(", &regex);
        replace_matches(&repo, r"fn (\w+)\(", "pub fn ${1}_v2(", &regex, &found).unwrap();
        assert_eq!(
            read(&repo, "a.txt"),
            "pub fn old_name_v2(a: i32)\nconcat cat cat\n"
        );

        let word = SearchOptions {
            whole_word: true,
            ..Default::default()
        };
        let found = targets(&repo, "cat", &word);
        replace_matches(&repo, "cat", "dog", &word, &found).unwrap();
        assert_eq!(read(&repo, "a.txt").lines().nth(1), Some("concat dog dog"));
    }

    #[test]
    fn replace_leaves_files_it_cannot_edit_safely_alone() {
        let directory = TestDirectory::new("search-skip");
        let repo = repository(&directory);
        write(&repo, "ok.txt", "old\n");
        fs::write(repo.join("latin1.txt"), b"old caf\xe9\n").unwrap();
        commit_all(&repo);
        let all = vec![
            Match {
                path: "latin1.txt".to_owned(),
                line: 1,
                text: "old caf".to_owned(),
            },
            Match {
                path: "ok.txt".to_owned(),
                line: 1,
                text: "old".to_owned(),
            },
            Match {
                path: "gone.txt".to_owned(),
                line: 1,
                text: "old".to_owned(),
            },
        ];
        let report = replace_matches(&repo, "old", "new", &SearchOptions::default(), &all).unwrap();
        assert_eq!(report.files, 1);
        assert_eq!(report.skipped_files, ["gone.txt", "latin1.txt"]);
        assert_eq!(fs::read(repo.join("latin1.txt")).unwrap(), b"old caf\xe9\n");
        assert_eq!(read(&repo, "ok.txt"), "new\n");
    }

    #[test]
    fn replace_refuses_paths_that_leave_the_repository() {
        let directory = TestDirectory::new("search-escape");
        let repo = repository(&directory);
        let outside = directory.path().join("outside.txt");
        fs::write(&outside, "old\n").unwrap();
        for path in ["../outside.txt", "/etc/passwd", ""] {
            let target = vec![Match {
                path: path.to_owned(),
                line: 1,
                text: "old".to_owned(),
            }];
            let error = replace_matches(&repo, "old", "new", &SearchOptions::default(), &target)
                .unwrap_err();
            assert!(error.contains("Refusing to touch"), "{path}: {error}");
        }
        assert_eq!(fs::read_to_string(outside).unwrap(), "old\n");
    }
}
