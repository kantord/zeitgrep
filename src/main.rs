use std::collections::{HashMap, HashSet};
use std::io;
use std::io::ErrorKind;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
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

    /// Suppress output; exit 0 if any match is found, 1 otherwise
    #[arg(short = 'q', long = "quiet")]
    quiet: bool,

    /// Controls when to use color
    #[arg(long, value_enum, default_value = "auto")]
    color: Color,

    /// Sorting order for frecency score ("asc" or "desc")
    #[arg(long, value_enum, default_value = "desc")]
    sort: SortOrder,
}

#[derive(Copy, Clone, Debug, ValueEnum, PartialEq)]
enum Color {
    Always,
    Auto,
    Never,
}

#[derive(Copy, Clone, Debug, ValueEnum, PartialEq)]
enum SortOrder {
    Asc,
    Desc,
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
/// With `stop_at_first`, workers quit as soon as any thread has recorded a
/// match (used by `--quiet`, where only the existence of a match matters).
fn find_matches(pattern: &str, stop_at_first: bool) -> Vec<MatchResult> {
    let matcher = RegexMatcher::new(pattern).expect("Invalid regular expression");
    let root = Path::new(".");
    let matches = Arc::new(Mutex::new(Vec::<MatchResult>::new()));
    let found = Arc::new(AtomicBool::new(false));

    WalkBuilder::new(root).build_parallel().run(|| {
        let matcher = matcher.clone();
        let matches_outer = matches.clone();
        let found_outer = found.clone();

        Box::new(move |result| {
            if found_outer.load(Ordering::Relaxed) {
                return ignore::WalkState::Quit;
            }

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
                let found_sink = found_outer.clone();

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
                        if stop_at_first {
                            found_sink.store(true, Ordering::Relaxed);
                            return Ok(false);
                        }
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

fn print_matches(matches: Vec<MatchResult>, pattern: &str, args: &Args) -> io::Result<()> {
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
            let line = m.line_text.trim_end_matches(&['\r', '\n'][..]).as_bytes();

            if args.score {
                write!(stdout, "{:.2}: ", m.frecency_score * 1e8)?;
            }
            write!(stdout, "{}:{}:", m.path.display(), m.line_number)?;
            if args.column {
                write!(stdout, "{}:", start + 1)?;
            }

            stdout.set_color(&normal)?;
            stdout.write_all(&line[..start])?;
            stdout.set_color(&highlight)?;
            stdout.write_all(&line[start..end])?;
            stdout.set_color(&normal)?;
            stdout.write_all(&line[end..])?;
            stdout.write_all(b"\n")?;
        }
    }

    Ok(())
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

    let mut matches = find_matches(&pattern_str, args.quiet);

    if args.quiet {
        std::process::exit(if matches.is_empty() { 1 } else { 0 });
    }

    calculate_frecencies(&mut matches)?;
    let mut sorted_matches = sort_matches(matches);

    if args.sort == SortOrder::Asc {
        sorted_matches.reverse();
    }

    match print_matches(sorted_matches, &pattern_str, &args) {
        Err(e) if e.kind() == ErrorKind::BrokenPipe => {
            // downstream closed early (e.g. `head`) → silent exit
            return Ok(());
        }
        Err(e) => {
            // real I/O error
            eprintln!("I/O error: {}", e);
            std::process::exit(1);
        }
        Ok(()) => {}
    }

    Ok(())
}
