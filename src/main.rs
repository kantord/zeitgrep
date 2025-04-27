use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use clap::Parser;
use git2::{Blame, BlameOptions, Repository};
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
    frecency_score: i32,
}

impl MatchResult {
    fn calculate_frecency<'repo>(
        &mut self,
        repo: &'repo Repository,
        blame_cache: &mut HashMap<PathBuf, Blame<'repo>>,
        commit_count_cache: &mut HashMap<PathBuf, i32>,
    ) -> Result<(), git2::Error> {
        let path = self
            .path
            .strip_prefix("./")
            .unwrap_or(&self.path)
            .to_path_buf();

        // Get blame for the file (from cache or fresh)
        let blame = blame_cache.entry(path.clone()).or_insert_with(|| {
            repo.blame_file(&path, Some(&mut BlameOptions::new()))
                .expect("Failed to blame file")
        });

        let line_idx = (self.line_number - 1) as usize;
        let hunk = blame.get_line(line_idx);

        // Line bonus
        let line_bonus = match hunk {
            Some(h) => {
                if h.final_commit_id().is_zero() {
                    5 // uncommitted line
                } else {
                    2 // committed line
                }
            }
            None => 0, // fallback
        };

        // Commit count for the file (from cache or computed)
        let file_score = *commit_count_cache.entry(path.clone()).or_insert_with(|| {
            let mut revwalk = repo.revwalk().expect("Couldn't create revwalk");
            revwalk.push_head().expect("Couldn't push HEAD");

            let mut count = 0;
            for oid_result in revwalk {
                if let Ok(oid) = oid_result {
                    if let Ok(commit) = repo.find_commit(oid) {
                        if commit.parents().len() > 1 {
                            continue; // Skip merge commits for simplicity
                        }
                        if let Some(tree) = commit.tree().ok() {
                            if tree.get_path(&path).is_ok() {
                                count += 1;
                            }
                        }
                    }
                }
            }
            count
        });

        self.frecency_score = file_score + line_bonus;
        Ok(())
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
                let path = entry.path();
                let matches = matches.clone();

                let mut searcher = SearcherBuilder::new()
                    .line_number(true)
                    .memory_map(unsafe { MmapChoice::auto() })
                    .binary_detection(BinaryDetection::quit(b'\x00'))
                    .build();

                let _ = searcher.search_path(
                    &matcher,
                    path,
                    UTF8(move |lnum, line| {
                        let match_result = MatchResult {
                            path: path.to_path_buf(),
                            line_number: lnum,
                            line_text: line.to_string(),
                            frecency_score: 0, // to be calculated later
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

    let matches = Arc::try_unwrap(matches)
        .expect("Arc still has multiple owners")
        .into_inner()
        .expect("Mutex poisoned");

    matches
}

fn calculate_frecencies(matches: &mut [MatchResult]) -> Result<(), git2::Error> {
    let repo = Repository::open(".")?;
    let mut blame_cache = HashMap::new();
    let mut commit_count_cache = HashMap::new();

    for m in matches {
        m.calculate_frecency(&repo, &mut blame_cache, &mut commit_count_cache)?;
    }

    Ok(())
}

fn sort_matches(mut matches: Vec<MatchResult>) -> Vec<MatchResult> {
    matches.sort_by_key(|m| -m.frecency_score);
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
            let start = matched.start();
            let end = matched.end();

            if show_score {
                write!(&mut stdout, "{:.2}: ", m.frecency_score as f32).unwrap();
            }

            write!(&mut stdout, "{}:{}:", m.path.display(), m.line_number).unwrap();

            stdout.set_color(&normal).unwrap();
            stdout.write_all(&m.line_text.as_bytes()[..start]).unwrap();

            stdout.set_color(&highlight).unwrap();
            stdout
                .write_all(&m.line_text.as_bytes()[start..end])
                .unwrap();

            stdout.set_color(&normal).unwrap();
            stdout.write_all(&m.line_text.as_bytes()[end..]).unwrap();

            stdout.flush().unwrap();
        } else {
            panic!("Failed to find match in line text");
        }
    }
}

fn main() {
    let args = Args::parse();
    let mut matches = find_matches(&args.pattern);

    if let Err(e) = calculate_frecencies(&mut matches) {
        eprintln!("Error calculating frecency: {}", e);
    }

    let sorted_matches = sort_matches(matches);
    print_matches(sorted_matches, &args.pattern, args.score);
}
