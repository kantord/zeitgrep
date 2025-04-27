use std::path::Path;

use clap::Parser;
use grep::{
    regex::RegexMatcher,
    searcher::{SearcherBuilder, sinks::UTF8},
};
use ignore::WalkBuilder;

/// Search frecently edited code in a Git repository
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Regular Expression pattern
    #[arg()]
    pattern: String,
}

fn main() {
    let args = Args::parse();

    let matcher = RegexMatcher::new(&args.pattern).expect("Invalid regular expressoin");
    let mut searcher = SearcherBuilder::new().line_number(true).build();

    let root = Path::new(".");

    for result in WalkBuilder::new(root).build() {
        let entry = result.expect("Failed to read directory entry");
        if entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
            let path = entry.path();
            if let Err(err) = searcher.search_path(
                &matcher,
                path,
                UTF8(|lnum, line| {
                    print!("{}:{}: {}", path.display(), lnum, line);
                    Ok(true)
                }),
            ) {
                eprintln!("Failed to search {}: {}", path.display(), err);
            }
        }
    }
}
