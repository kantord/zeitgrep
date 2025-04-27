use std::path::Path;

use clap::Parser;
use grep::{
    printer::{ColorSpecs, StandardBuilder},
    regex::RegexMatcher,
    searcher::{BinaryDetection, MmapChoice, SearcherBuilder},
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
    let color_specs =
        ColorSpecs::new(&["match:fg:red".parse().expect("Could not create color spec")]);

    let root = Path::new(".");

    WalkBuilder::new(root).build_parallel().run(|| {
        let matcher = matcher.clone();
        let mut searcher = SearcherBuilder::new()
            .line_number(true)
            .binary_detection(BinaryDetection::quit(b'\x00'))
            .memory_map(unsafe { MmapChoice::auto() })
            .build();
        let mut printer = StandardBuilder::new()
            .color_specs(color_specs.clone())
            .build(StandardStream::stdout(ColorChoice::Auto));

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
                if let Err(err) =
                    searcher.search_path(&matcher, path, printer.sink_with_path(&matcher, path))
                {
                    eprintln!("Failed to search {}: {}", path.display(), err);
                }
            }
            ignore::WalkState::Continue
        })
    });
}

