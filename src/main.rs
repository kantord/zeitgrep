use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use clap::Parser;
use git2::Repository;
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

/// Structure representing a single match found.
#[derive(Debug, Clone)]
struct MatchResult {
    path: PathBuf,
    line_number: u64,
    line_text: String,
    frecency_score: f32,
}

/// Get the current timestamp in seconds since epoch
fn current_timestamp() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("Time went backwards")
        .as_secs() as i64
}

/// Estimate number of lines from blob size
fn estimate_line_count(blob_size: usize) -> usize {
    (blob_size / 50).max(1) // Assume ~50 bytes per line
}

/// Remove a leading "./" component if present
fn normalize_repo_path(path: &Path) -> &Path {
    if let Ok(stripped) = path.strip_prefix(".") {
        stripped
    } else {
        path
    }
}

fn find_matches(pattern: &str) -> Vec<MatchResult> {
    let matcher = RegexMatcher::new(pattern).expect("Invalid regular expression");
    let root = Path::new(".");
    let matches = Arc::new(Mutex::new(Vec::<MatchResult>::new()));

    WalkBuilder::new(root).build_parallel().run(|| {
        let matcher = matcher.clone();
        let matches = matches.clone();

        Box::new(move |result| {
            let entry = match result {
                Ok(entry) => entry,
                Err(err) => {
                    eprintln!("Walk error: {}", err);
                    return ignore::WalkState::Continue;
                }
            };

            if entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
                let file_path = entry.into_path();
                let matches = matches.clone();

                let mut searcher = SearcherBuilder::new()
                    .line_number(true)
                    .memory_map(unsafe { MmapChoice::auto() })
                    .binary_detection(BinaryDetection::quit(b'\0'))
                    .build();

                let path_for_search = file_path.as_path();
                let path_in_closure = file_path.clone();

                let _ = searcher.search_path(
                    &matcher,
                    path_for_search,
                    UTF8(move |lnum, line| {
                        let match_result = MatchResult {
                            path: path_in_closure.clone(),
                            line_number: lnum,
                            line_text: line.to_string(),
                            frecency_score: 0.0,
                        };

                        let mut matches = matches.lock().unwrap();
                        matches.push(match_result);
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

fn calculate_frecencies(matches: &mut [MatchResult]) -> Result<(), git2::Error> {
    let repo = Repository::open(".")?;
    let workdir = repo.workdir().unwrap_or(Path::new("."));
    let now = current_timestamp();

    let mut paths_of_interest: HashSet<PathBuf> = HashSet::new();
    for m in matches.iter() {
        let rel = if m.path.is_absolute() {
            m.path.strip_prefix(workdir).unwrap_or(&m.path)
        } else {
            m.path.as_path()
        };
        paths_of_interest.insert(normalize_repo_path(rel).to_path_buf());
    }

    if paths_of_interest.is_empty() {
        return Err(git2::Error::from_str("no paths inside repository detected"));
    }

    let mut file_scores: HashMap<PathBuf, f32> =
        paths_of_interest.iter().map(|p| (p.clone(), 0.0)).collect();

    let mut blob_line_cache = HashMap::new();

    let mut revwalk = repo.revwalk()?;
    revwalk.push_head()?;
    let _ = revwalk.simplify_first_parent();

    let mut any_contrib = false;

    for oid in revwalk {
        let oid = oid?;
        let commit = repo.find_commit(oid)?;
        if commit.parents().len() > 1 {
            continue; // Skip merge commits
        }
        let tree = commit.tree()?;

        for path in &paths_of_interest {
            if let Ok(entry) = tree.get_path(path) {
                let blob_id = entry.id();
                let line_count = *blob_line_cache.entry(blob_id).or_insert_with(|| {
                    repo.find_blob(blob_id)
                        .map(|b| estimate_line_count(b.size()))
                        .unwrap_or(1)
                });

                let commit_time = commit.time().seconds();
                let age_seconds = (now - commit_time).max(1);
                let age_days = age_seconds as f32 / 86_400.0;
                let contribution = 1.0 / (line_count as f32 * age_days);
                if contribution > 0.0 {
                    any_contrib = true;
                }
                *file_scores.get_mut(path).unwrap() += contribution;
            }
        }
    }

    if !any_contrib {
        return Err(git2::Error::from_str(
            "no frecency contributions were accumulated (path handling still off)",
        ));
    }

    for m in matches {
        let rel = if m.path.is_absolute() {
            m.path.strip_prefix(workdir).unwrap_or(&m.path)
        } else {
            m.path.as_path()
        };
        let rel = normalize_repo_path(rel);
        m.frecency_score = *file_scores.get(rel).unwrap_or(&0.0);
    }

    Ok(())
}

fn sort_matches(mut matches: Vec<MatchResult>) -> Vec<MatchResult> {
    matches.sort_by(|a, b| {
        b.frecency_score
            .partial_cmp(&a.frecency_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    matches
}

fn print_matches(matches: Vec<MatchResult>, pattern: &str, show_score: bool) {
    let matcher = RegexMatcher::new(pattern).expect("Invalid regular expression");
    let mut stdout = StandardStream::stdout(ColorChoice::Auto);
    let mut normal = ColorSpec::new();
    normal.set_fg(None);

    let mut highlight = ColorSpec::new();
    highlight.set_fg(Some(termcolor::Color::Red)).set_bold(true);

    for m in matches {
        if let Ok(Some(matched)) = matcher.find(m.line_text.as_bytes()) {
            let (start, end) = (matched.start(), matched.end());

            let line_clean = m.line_text.trim_end_matches(&['\n', '\n'][..]);
            let bytes = line_clean.as_bytes();

            if show_score {
                let display_score = m.frecency_score * 1e8; 
                write!(&mut stdout, "{:.2}: ", display_score).unwrap();
            }

            write!(&mut stdout, "{}:{}:", m.path.display(), m.line_number).unwrap();

            stdout.set_color(&normal).unwrap();
            stdout.write_all(&bytes[..start]).unwrap();

            stdout.set_color(&highlight).unwrap();
            stdout.write_all(&bytes[start..end]).unwrap();

            stdout.set_color(&normal).unwrap();
            stdout.write_all(&bytes[end..]).unwrap();
            stdout
                .write_all(
                    b"
",
                )
                .unwrap();
        }
    }
}
fn main() {
    let args = Args::parse();
    let mut matches = find_matches(&args.pattern);

    match calculate_frecencies(&mut matches) {
        Ok(()) => {}
        Err(e) => {
            eprintln!("Error calculating frecency: {}", e);
            std::process::exit(1);
        }
    }

    let sorted_matches = sort_matches(matches);
    print_matches(sorted_matches, &args.pattern, args.score);
}

