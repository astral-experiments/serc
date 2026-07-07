use std::path::Path;
use std::process::Command;

fn git(checkout: &Path, arguments: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(checkout)
        .args(arguments)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
}

#[test]
fn pinned_corpus_validation_rejects_ignored_non_python_artifacts() {
    let checkout = tempfile::tempdir().unwrap();
    let corpus = checkout.path().join("numpy");
    fs_err::create_dir(&corpus).unwrap();
    fs_err::write(checkout.path().join(".gitignore"), "numpy/*.txt\n").unwrap();
    fs_err::write(corpus.join("tracked.py"), "pass\n").unwrap();

    git(checkout.path(), &["init", "--quiet"]);
    git(checkout.path(), &["config", "user.name", "Serc Tests"]);
    git(
        checkout.path(),
        &["config", "user.email", "serc@example.com"],
    );
    git(checkout.path(), &["add", ".gitignore", "numpy/tracked.py"]);
    git(checkout.path(), &["commit", "--quiet", "-m", "fixture"]);

    let validator =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/benchmarks/validate-corpus.sh");
    let clean = Command::new("bash")
        .arg(&validator)
        .arg(checkout.path())
        .arg("numpy")
        .output()
        .unwrap();
    assert!(clean.status.success(), "{clean:?}");

    fs_err::write(corpus.join("artifact.txt"), "affects traversal\n").unwrap();
    let dirty = Command::new("bash")
        .arg(validator)
        .arg(checkout.path())
        .arg("numpy")
        .output()
        .unwrap();

    assert!(!dirty.status.success(), "{dirty:?}");
    assert!(
        String::from_utf8_lossy(&dirty.stderr).contains("numpy/artifact.txt"),
        "{dirty:?}"
    );
}
