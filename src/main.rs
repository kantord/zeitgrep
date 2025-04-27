use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use clap::Parser;
use grep::matcher::Matcher;
use grep::{
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
}

/// Structure representing a single match found.
#[derive(Debug, Clone)]
struct MatchResult {
    path: PathBuf,
    line_number: u64,
    line_text: String,
    frecency_score: i32,
}

fn sort_matches(mut matches: Vec<MatchResult>) -> Vec<MatchResult> {
    matches.sort_by_key(|m| m.frecency_score);
    matches
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
                        let mut matches = matches.lock().unwrap();
                        matches.push(MatchResult {
                            path: path.to_path_buf(),
                            line_number: lnum,
                            line_text: line.to_string(),
                            frecency_score: 0,
                        });
                        Ok(true)
                    }),
                );
            }

            ignore::WalkState::Continue
        })
    });

    // After all threads complete, get matches
    let matches = Arc::try_unwrap(matches)
        .expect("Arc still has multiple owners")
        .into_inner()
        .expect("Mutex poisoned");

    matches
}

fn print_matches(matches: Vec<MatchResult>, pattern: &str) {
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
    let matches = find_matches(&args.pattern);
    let sorted_matches = sort_matches(matches);
    print_matches(sorted_matches, &args.pattern);
}

