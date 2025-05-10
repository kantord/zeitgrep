use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Error, Result};
use clap::{Parser, ValueEnum};
use frecenfile::analyze_repo;
use grep::{
    matcher::Matcher,
    regex::RegexMatcher,
    searcher::{BinaryDetection, MmapChoice, SearcherBuilder, sinks::UTF8},
};
use ignore::WalkBuilder;
use termcolor::{ColorChoice, ColorSpec, StandardStream, WriteColor};

/// Search frecently edited code in a Git repository
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Regular Expression pattern
    #[arg()]
    pattern: String,

    /// Case-insensitive regex matching
    #[arg(short = 'i', long = "ignore-case")]
    ignore_case: bool,

    /// Case-insensitive unless the pattern has an uppercase letter
    #[arg(short = 'S', long = "smart-case")]
    smart_case: bool,

    /// Show frecency scores in output
    #[arg(long)]
    score: bool,

    /// Show column number of matches
    #[arg(long)]
    column: bool,

    /// Controls when to use color
    #[arg(long, value_enum, default_value = "auto")]
    color: Color,
}

#[derive(Copy, Clone, Debug, ValueEnum, PartialEq)]
enum Color {
    Always,
    Auto,
    Never,
}

/// A single line‑match.
#[derive(Debug, Clone)]
struct MatchResult {
    path: PathBuf,
    line_number: u64,
    line_text: String,
    frecency_score: f32,
}

/// Remove a leading “./” component if present (cosmetic).
fn normalize_repo_path(path: &Path) -> &Path {
    path.strip_prefix(".").unwrap_or(path)
}

/// Run ripgrep‑style search over the working tree.
fn find_matches(pattern: &str) -> Vec<MatchResult> {
    let matcher = RegexMatcher::new(pattern).expect("Invalid regular expression");
    let root = Path::new(".");
    let matches = Arc::new(Mutex::new(Vec::<MatchResult>::new()));

    WalkBuilder::new(root).build_parallel().run(|| {
        let matcher = matcher.clone();
        let matches_outer = matches.clone();

        Box::new(move |result| {
            let entry = match result {
                Ok(e) => e,
                Err(err) => {
                    eprintln!("Walk error: {err}");
                    return ignore::WalkState::Continue;
                }
            };

            if entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
                let path_for_search = entry.path().to_path_buf();
                let path_for_vec = entry.path().to_path_buf();

                let matches_inner = matches_outer.clone();

                let mut searcher = SearcherBuilder::new()
                    .line_number(true)
                    .memory_map(unsafe { MmapChoice::auto() })
                    .binary_detection(BinaryDetection::quit(b'\0'))
                    .build();

                let _ = searcher.search_path(
                    &matcher,
                    &path_for_search,
                    UTF8(move |lnum, line| {
                        let mut vec_guard = matches_inner.lock().unwrap();
                        vec_guard.push(MatchResult {
                            path: path_for_vec.clone(),
                            line_number: lnum,
                            line_text: line.to_string(),
                            frecency_score: 0.0,
                        });
                        Ok(true)
                    }),
                );
            }

            ignore::WalkState::Continue
        })
    });

    Arc::try_unwrap(matches)
        .expect("Arc still has multiple owners")
        .into_inner()
        .expect("Mutex poisoned")
}

/// Annotate each match with a frecency score from `frecenfile`.
/// Files not returned by the library receive score 0.0.
fn calculate_frecencies(matches: &mut [MatchResult]) -> Result<(), Error> {
    let paths_of_interest: HashSet<PathBuf> = matches
        .iter()
        .map(|m| normalize_repo_path(&m.path).to_path_buf())
        .collect();

    if paths_of_interest.is_empty() {
        return Ok(());
    }

    let scores_vec = analyze_repo(Path::new("."), Some(paths_of_interest.clone()), Some(3000))?;

    let score_map: HashMap<PathBuf, f32> =
        scores_vec.into_iter().map(|(p, s)| (p, s as f32)).collect();

    for m in matches.iter_mut() {
        let rel = normalize_repo_path(&m.path);
        m.frecency_score = *score_map.get(rel).unwrap_or(&0.0);
    }

    Ok(())
}

/// Sort highest‑score first (ties keep original order).
fn sort_matches(mut matches: Vec<MatchResult>) -> Vec<MatchResult> {
    matches.sort_by(|a, b| {
        b.frecency_score
            .partial_cmp(&a.frecency_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    matches
}

/// Pretty‑print results with optional score column.
fn print_matches(matches: Vec<MatchResult>, pattern: &str, args: Args) {
    let matcher = RegexMatcher::new(pattern).expect("Invalid regular expression");
    let color_choice = match args.color {
        Color::Always => ColorChoice::Always,
        Color::Never => ColorChoice::Never,
        Color::Auto => ColorChoice::Auto,
    };
    let mut stdout = StandardStream::stdout(color_choice);

    let normal = ColorSpec::new();
    let mut highlight = ColorSpec::new();
    highlight.set_fg(Some(termcolor::Color::Red)).set_bold(true);

    for m in matches {
        if let Ok(Some(matched)) = matcher.find(m.line_text.as_bytes()) {
            let (start, end) = (matched.start(), matched.end());
            let line_clean = m.line_text.trim_end_matches(&['\r', '\n'][..]);
            let bytes = line_clean.as_bytes();

            if args.score {
                write!(stdout, "{:.2}: ", m.frecency_score * 1e8).unwrap();
            }

            write!(stdout, "{}:{}:", m.path.display(), m.line_number).unwrap();

            if args.column {
                write!(stdout, "{}:", start + 1).unwrap();
            }

            stdout.set_color(&normal).unwrap();
            stdout.write_all(&bytes[..start]).unwrap();

            stdout.set_color(&highlight).unwrap();
            stdout.write_all(&bytes[start..end]).unwrap();

            stdout.set_color(&normal).unwrap();
            stdout.write_all(&bytes[end..]).unwrap();
            stdout.write_all(b"\n").unwrap();
        }
    }
}

fn main() -> Result<()> {
    let args = Args::parse();
    let case_insensitive =
        args.ignore_case || (args.smart_case && args.pattern.to_lowercase() == args.pattern);
    let pattern_str = if case_insensitive {
        format!("(?i){}", args.pattern)
    } else {
        args.pattern.clone()
    };

    let mut matches = find_matches(&pattern_str);

    calculate_frecencies(&mut matches)?;

    let sorted_matches = sort_matches(matches);
    print_matches(sorted_matches, &pattern_str, args);

    Ok(())
}

#[cfg(test)]
mod tests {
    mod test_utils;
    use test_utils::{create_mock_repo, run_zg};

    #[test]
    fn sorts_files_correctly_based_on_frecency() {
        let repo_dir = create_mock_repo(&[("alpha.rs", 5), ("beta.rs", 3), ("gamma.rs", 1)]);
        let stdout = run_zg(&repo_dir, &["println!"]);

        let pos = |needle: &str| stdout.find(needle).expect(needle);

        let p_alpha = pos("alpha.rs");
        let p_beta = pos("beta.rs");
        let p_gamma = pos("gamma.rs");

        assert!(
            p_alpha < p_beta && p_beta < p_gamma,
            "order wrong:\n{stdout}"
        );
    }

    #[test]
    fn placeholder_scores_match_stdout() {
        let dir = create_mock_repo(&[("alpha.rs", 5), ("beta.rs", 3), ("gamma.rs", 1)]);
        let stdout = run_zg(&dir, &["println!", "--score"]);

        let expected = [
            ("alpha.rs", 419238720.0_f32),
            ("beta.rs", 252082560.0_f32),
            ("gamma.rs", 83847740.0_f32),
        ];
        for (file, want) in expected {
            let line = stdout.lines().find(|l| l.contains(file)).expect(file);
            let score_str = line.split(':').next().unwrap();
            let got: f32 = score_str.parse().unwrap();
            assert!((got - want).abs() < 1e-3, "{file}: got {got}, want {want}");
        }
    }

    #[test]
    fn column_flag_outputs_correct_columns() {
        let dir = create_mock_repo(&[("main.rs", 1), ("lib.rs", 1)]);

        std::fs::write(
            dir.path().join("main.rs"),
            "fn main() {\n    println!(\"main\");\n}\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("lib.rs"),
            "fn lib() {\nprintln!(\"lib\");\n}\n",
        )
        .unwrap();

        let stdout = run_zg(&dir, &["println!", "--column"]);
        let mut found_main = false;
        let mut found_lib = false;

        for line in stdout.lines() {
            let mut parts = line.splitn(4, ':');
            let file = parts.next().unwrap_or("");
            let line_no = parts.next().unwrap_or("");
            let col_no = parts.next().unwrap_or("");
            let _rest = parts.next().unwrap_or("");

            if !line_no.is_empty() && !col_no.is_empty() {
                let col: usize = col_no.parse().expect("valid column number");

                if file.contains("main.rs") {
                    assert_eq!(col, 5, "wrong column number for main.rs");
                    found_main = true;
                } else if file.contains("lib.rs") {
                    assert_eq!(col, 1, "wrong column number for lib.rs");
                    found_lib = true;
                }
            }
        }

        assert!(found_main, "no output for main.rs");
        assert!(found_lib, "no output for lib.rs");
    }

    #[test]
    fn color_never_disables_colored_output() {
        let dir = create_mock_repo(&[("main.rs", 1)]);
        std::fs::write(
            dir.path().join("main.rs"),
            "fn main() {\n    println!(\"main\");\n}\n",
        )
        .unwrap();
        let stdout = run_zg(&dir, &["println!", "--color=never"]);

        // ANSI escape sequences for color usually start with \x1b (ESC)
        assert!(
            !stdout.contains('\x1b'),
            "Expected no ANSI escape codes in output, got:\n{stdout}"
        );
    }

    #[test]
    fn color_always_shows_color_output() {
        let dir = create_mock_repo(&[("main.rs", 1)]);
        std::fs::write(
            dir.path().join("main.rs"),
            "fn main() {\n    println!(\"main\");\n}\n",
        )
        .unwrap();
        let stdout = run_zg(&dir, &["println!", "--color=always"]);

        // ANSI escape sequences for color usually start with \x1b (ESC)
        assert!(
            stdout.contains('\x1b'),
            "Expected ANSI escape codes in output, none found"
        );
    }

    #[test]
    fn default_search_is_case_sensitive() {
        let dir = create_mock_repo(&[("case.rs", 1)]);
        let file_path = dir.path().join("case.rs");
        std::fs::write(&file_path, "Hello\nhello\nHELLO\n").unwrap();

        let stdout = run_zg(&dir, &["hello"]);

        assert!(
            stdout.contains("case.rs:2"),
            "Expected match for lowercase 'hello', got:\n{}",
            stdout
        );
        assert!(
            !stdout.contains("case.rs:1"),
            "Unexpected match for 'Hello'"
        );
        assert!(
            !stdout.contains("case.rs:3"),
            "Unexpected match for 'HELLO'"
        );
    }

    #[test]
    fn ignore_case_short_flag_matches_all_cases() {
        let dir = create_mock_repo(&[("case.rs", 1)]);
        let file_path = dir.path().join("case.rs");
        std::fs::write(&file_path, "Hello\nhello\nHELLO\n").unwrap();

        let stdout = run_zg(&dir, &["-i", "hello"]);

        assert!(stdout.contains("case.rs:1"), "Expected match for 'Hello'");
        assert!(stdout.contains("case.rs:2"), "Expected match for 'hello'");
        assert!(stdout.contains("case.rs:3"), "Expected match for 'HELLO'");
    }

    #[test]
    fn smart_case_no_uppercase_letters() {
        let dir = create_mock_repo(&[("case.rs", 1)]);
        let file_path = dir.path().join("case.rs");
        std::fs::write(&file_path, "Hello\nhello\nHELLO\n").unwrap();

        let stdout = run_zg(&dir, &["-S", "hello"]);

        assert!(stdout.contains("case.rs:1"), "Expected match for 'Hello'");
        assert!(stdout.contains("case.rs:2"), "Expected match for 'hello'");
        assert!(stdout.contains("case.rs:3"), "Expected match for 'HELLO'");
    }

    #[test]
    fn smart_case_has_uppercase_letters() {
        let dir = create_mock_repo(&[("case.rs", 1)]);
        let file_path = dir.path().join("case.rs");
        std::fs::write(&file_path, "Hello\nhello\nHELLO\n").unwrap();

        let stdout = run_zg(&dir, &["--smart-case", "Hello"]);

        assert!(stdout.contains("case.rs:1"), "Expected match for 'Hello'");
        assert!(
            !stdout.contains("case.rs:2"),
            "Unexpected match for 'hello'"
        );
        assert!(
            !stdout.contains("case.rs:3"),
            "Unexpected match for 'HELLO'"
        );
    }

    #[test]
    fn ignore_case_long_flag_matches_all_cases() {
        let dir = create_mock_repo(&[("case.rs", 1)]);
        let file_path = dir.path().join("case.rs");
        std::fs::write(&file_path, "Hello\nhello\nHELLO\n").unwrap();

        let stdout = run_zg(&dir, &["--ignore-case", "HelLo"]);

        assert!(stdout.contains("case.rs:1"), "Expected match for 'Hello'");
        assert!(stdout.contains("case.rs:2"), "Expected match for 'hello'");
        assert!(stdout.contains("case.rs:3"), "Expected match for 'HELLO'");
    }
}
