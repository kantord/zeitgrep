use assert_cmd::Command;
use git2::Repository;
use std::{fs::File, io::Write, path::Path};
use tempfile::TempDir;

/// Create a temporary Git repo and make `commit_count` commits to each file in `spec`.
pub fn create_mock_repo(spec: &[(&str, usize)]) -> TempDir {
    let dir = tempfile::tempdir().expect("failed to create tempdir");
    let repo = Repository::init(dir.path()).expect("failed to init repo");
    let sig = repo.signature().unwrap();

    for (file_name, commit_count) in spec {
        let file_path = dir.path().join(file_name);

        for n in 0..*commit_count {
            // overwrite the file
            {
                let mut f = File::create(&file_path).unwrap();
                writeln!(f, "fn f_{n}() {{ println!(\"{file_name} #{n}\"); }}").unwrap();
            }

            // stage, write tree, and commit
            let mut idx = repo.index().unwrap();
            idx.add_path(Path::new(file_name)).unwrap();
            idx.write().unwrap();
            let tree_id = idx.write_tree().unwrap();
            let tree = repo.find_tree(tree_id).unwrap();

            let parents = repo
                .head()
                .ok()
                .and_then(|h| {
                    if h.is_branch() {
                        Some(vec![repo.find_commit(h.target().unwrap()).unwrap()])
                    } else {
                        None
                    }
                })
                .unwrap_or_default();
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

pub fn run_zg(repo_dir: &TempDir, args: &[&str]) -> String {
    let assert = Command::cargo_bin("zg")
        .expect("binary `zg` not found")
        .current_dir(repo_dir.path())
        .args(args)
        .assert()
        .success();
    let out = assert.get_output().stdout.clone();
    String::from_utf8_lossy(&out).into_owned()
}
