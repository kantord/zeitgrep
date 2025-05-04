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

#[cfg(test)]
mod tests {
    use assert_cmd::Command;
    use git2::Repository;
    use std::{fs::File, io::Write, path::Path};
    use tempfile::TempDir;

    fn create_mock_repo(spec: &[(&str, usize)]) -> TempDir {
        let dir = tempfile::tempdir().expect("tmp dir");
        let repo = Repository::init(dir.path()).expect("init repo");
        let sig = repo.signature().unwrap();

        for (file_name, commit_count) in spec {
            let file_path = dir.path().join(file_name);

            for n in 0..*commit_count {
                {
                    let mut f = File::create(&file_path).unwrap();
                    writeln!(f, "fn f_{n}() {{ println!(\"{file_name} #{n}\"); }}").unwrap();
                }

                let mut idx = repo.index().unwrap();
                idx.add_path(Path::new(file_name)).unwrap();
                idx.write().unwrap();
                let tree_id = idx.write_tree().unwrap();
                let tree = repo.find_tree(tree_id).unwrap();

                let parents = if let Ok(head) = repo.head() {
                    if head.is_branch() {
                        vec![repo.find_commit(head.target().unwrap()).unwrap()]
                    } else {
                        vec![]
                    }
                } else {
                    vec![]
                };

                let parent_refs: Vec<&git2::Commit> = parents.iter().collect();
                repo.commit(
                    Some("HEAD"),
                    &sig,
                    &sig,
                    &format!("commit {n} for {file_name}"),
                    &tree,
                    &parent_refs,
                )
                .unwrap();
            }
        }
        dir
    }

    #[test]
    fn sorts_files_correctly_based_on_frecency() {
        let repo_dir = create_mock_repo(&[("alpha.rs", 5), ("beta.rs", 3), ("gamma.rs", 1)]);

        let output = Command::cargo_bin("zg")
            .unwrap()
            .current_dir(repo_dir.path())
            .arg("println!")
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();

        let stdout = String::from_utf8_lossy(&output);

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
        let output = Command::cargo_bin("zg")
            .unwrap()
            .current_dir(dir.path())
            .args(["println!", "--score"])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        let stdout = String::from_utf8_lossy(&output);
        let expected = [
            ("alpha.rs", 500000000.0_f32),
            ("beta.rs", 300000000.0_f32),
            ("gamma.rs", 100000000.0_f32),
        ];
        for (file, want) in expected {
            let line = stdout.lines().find(|l| l.contains(file)).expect(file);
            let score_str = line.split(':').next().unwrap();
            let got: f32 = score_str.parse().unwrap();
            assert!((got - want).abs() < 1e-3, "{file}: got {got}, want {want}");
        }
    }
}
