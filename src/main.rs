use std::path::Path;

use clap::Parser;
use grep::{
    printer::{ColorSpecs, StandardBuilder},
    regex::RegexMatcher,
    searcher::SearcherBuilder,
};
use ignore::WalkBuilder;
use termcolor::{ColorChoice, StandardStream};

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

    let matcher = RegexMatcher::new(&args.pattern).expect("Invalid regular expression");
    let mut searcher = SearcherBuilder::new().line_number(true).build();
    let color_specs =
        ColorSpecs::new(&["match:fg:red".parse().expect("Could not create color spec")]);
    let mut printer = StandardBuilder::new()
        .color_specs(color_specs)
        .build(StandardStream::stdout(ColorChoice::Auto));

    let root = Path::new(".");

    for result in WalkBuilder::new(root).build() {
        let entry = result.expect("Failed to read directory entry");
        if entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
            let path = entry.path();
            if let Err(err) =
                searcher.search_path(&matcher, path, printer.sink_with_path(&matcher, path))
            {
                eprintln!("Failed to search {}: {}", path.display(), err);
            }
        }
    }
}

