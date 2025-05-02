use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use clap::Parser;
use frecenfile::analyze_repo;
use git2::Error as GitError;
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

    /// Show frecency scores in output
    #[arg(long)]
    score: bool,
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
fn calculate_frecencies(matches: &mut [MatchResult]) -> Result<(), GitError> {
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
fn print_matches(matches: Vec<MatchResult>, pattern: &str, show_score: bool) {
    let matcher = RegexMatcher::new(pattern).expect("Invalid regular expression");
    let mut stdout = StandardStream::stdout(ColorChoice::Auto);

    let normal = ColorSpec::new();
    let mut highlight = ColorSpec::new();
    highlight.set_fg(Some(termcolor::Color::Red)).set_bold(true);

    for m in matches {
        if let Ok(Some(matched)) = matcher.find(m.line_text.as_bytes()) {
            let (start, end) = (matched.start(), matched.end());
            let line_clean = m.line_text.trim_end_matches(&['\r', '\n'][..]);
            let bytes = line_clean.as_bytes();

            if show_score {
                write!(stdout, "{:.2}: ", m.frecency_score * 1e8).unwrap();
            }

            write!(stdout, "{}:{}:", m.path.display(), m.line_number).unwrap();

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

fn main() {
    let args = Args::parse();

    let mut matches = find_matches(&args.pattern);

    if let Err(e) = calculate_frecencies(&mut matches) {
        eprintln!("Error calculating frecency: {e}");
        std::process::exit(1);
    }

    let sorted_matches = sort_matches(matches);
    print_matches(sorted_matches, &args.pattern, args.score);
}

