mod common;
use assert_cmd::Command;
use common::{create_mock_repo, run_zg};

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
fn sorts_files_correctly_ascending() {
    let repo_dir = create_mock_repo(&[("alpha.rs", 5), ("beta.rs", 3), ("gamma.rs", 1)]);
    let stdout = run_zg(&repo_dir, &["println!", "--sort=asc"]);

    let pos = |needle: &str| stdout.find(needle).expect(needle);

    let p_gamma = pos("gamma.rs");
    let p_beta = pos("beta.rs");
    let p_alpha = pos("alpha.rs");

    assert!(
        p_gamma < p_beta && p_beta < p_alpha,
        "ascending order wrong:\n{stdout}"
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

#[test]
fn quiet_suppresses_output_and_exits_zero_on_match() {
    let dir = create_mock_repo(&[("alpha.rs", 1)]);
    let assert = Command::cargo_bin("zg")
        .expect("binary `zg` not found")
        .current_dir(dir.path())
        .args(["--quiet", "println!"])
        .assert()
        .success();

    assert!(
        assert.get_output().stdout.is_empty(),
        "Expected no stdout in quiet mode, got: {:?}",
        String::from_utf8_lossy(&assert.get_output().stdout)
    );
}

#[test]
fn quiet_short_flag_exits_one_when_no_match() {
    let dir = create_mock_repo(&[("alpha.rs", 1)]);
    let assert = Command::cargo_bin("zg")
        .expect("binary `zg` not found")
        .current_dir(dir.path())
        .args(["-q", "no_such_pattern_xyz"])
        .assert()
        .failure();

    assert!(
        assert.get_output().stdout.is_empty(),
        "Expected no stdout in quiet mode, got: {:?}",
        String::from_utf8_lossy(&assert.get_output().stdout)
    );
}
