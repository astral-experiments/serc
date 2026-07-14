use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn git(checkout: &Path, arguments: &[&str]) -> Output {
    Command::new("git")
        .arg("-C")
        .arg(checkout)
        .args(arguments)
        .output()
        .unwrap()
}

fn checked_git(checkout: &Path, arguments: &[&str]) {
    let output = git(checkout, arguments);
    assert!(output.status.success(), "{output:?}");
}

fn initialize_repository(checkout: &Path) {
    checked_git(checkout, &["init", "--quiet"]);
    checked_git(checkout, &["config", "user.name", "Serc Tests"]);
    checked_git(checkout, &["config", "user.email", "serc@example.com"]);
    checked_git(checkout, &["config", "commit.gpgSign", "false"]);
}

fn write_pins(checkout: &Path, revision: &str, lock_revision: &str) {
    fs_err::write(
        checkout.join("Cargo.toml"),
        format!(
            r#"[workspace.dependencies]
ruff_python_ast = {{ git = "https://github.com/astral-sh/ruff", rev = "{revision}" }}
ruff_python_parser = {{ git = "https://github.com/astral-sh/ruff", rev = "{revision}" }}
"#,
        ),
    )
    .unwrap();
    fs_err::write(
        checkout.join("Cargo.lock"),
        format!(
            r#"[[package]]
name = "ruff_python_ast"
source = "git+https://github.com/astral-sh/ruff?rev={lock_revision}#{lock_revision}"

[[package]]
name = "ruff_python_parser"
source = "git+https://github.com/astral-sh/ruff?rev={lock_revision}#{lock_revision}"
"#,
        ),
    )
    .unwrap();
}

fn pinned_checkout() -> (tempfile::TempDir, String) {
    let ruff = tempfile::tempdir().unwrap();
    initialize_repository(ruff.path());
    fs_err::write(ruff.path().join("README.md"), "Ruff fixture\n").unwrap();
    checked_git(ruff.path(), &["add", "README.md"]);
    checked_git(ruff.path(), &["commit", "--quiet", "-m", "fixture"]);
    let revision = String::from_utf8(git(ruff.path(), &["rev-parse", "HEAD"]).stdout)
        .unwrap()
        .trim()
        .to_string();

    let checkout = tempfile::tempdir().unwrap();
    initialize_repository(checkout.path());
    write_pins(checkout.path(), &revision, &revision);
    checked_git(
        checkout.path(),
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            "--quiet",
            ruff.path().to_str().unwrap(),
            "vendor/ruff",
        ],
    );
    checked_git(checkout.path(), &["add", "."]);
    checked_git(checkout.path(), &["commit", "--quiet", "-m", "fixture"]);

    (checkout, revision)
}

fn checker() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/check-ruff-pin")
}

fn check(checkout: &Path) -> Output {
    Command::new("bash")
        .arg(checker())
        .args(["--repo-root", checkout.to_str().unwrap()])
        .output()
        .unwrap()
}

#[test]
fn ruff_pin_check_accepts_coherent_inputs_and_rejects_a_stale_lockfile() {
    let (checkout, revision) = pinned_checkout();
    let output = check(checkout.path());
    assert!(output.status.success(), "{output:?}");

    let stale_revision = "1111111111111111111111111111111111111111";
    assert_ne!(revision, stale_revision);
    write_pins(checkout.path(), &revision, stale_revision);
    let output = check(checkout.path());
    assert!(!output.status.success(), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("Cargo.lock Ruff revision"),
        "{output:?}"
    );
}

#[test]
fn ruff_pin_check_rejects_an_uninitialized_submodule() {
    let (checkout, _) = pinned_checkout();
    fs_err::remove_file(checkout.path().join("vendor/ruff/.git")).unwrap();

    let output = check(checkout.path());
    assert!(!output.status.success(), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("submodule is not initialized"),
        "{output:?}"
    );
}

#[test]
fn ruff_pin_check_rejects_local_corpus_changes() {
    let (checkout, _) = pinned_checkout();
    let ruff = checkout.path().join("vendor/ruff");

    fs_err::write(ruff.join("README.md"), "modified fixture\n").unwrap();
    let output = check(checkout.path());
    assert!(!output.status.success(), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("local changes"),
        "{output:?}"
    );

    checked_git(&ruff, &["restore", "README.md"]);
    fs_err::write(ruff.join("untracked.py"), "fixture = True\n").unwrap();
    let output = check(checkout.path());
    assert!(!output.status.success(), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("local changes"),
        "{output:?}"
    );
}
