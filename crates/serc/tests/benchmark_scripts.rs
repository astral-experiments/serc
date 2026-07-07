use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

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

#[test]
#[cfg(unix)]
fn cargo_artifact_parser_uses_isolated_no_site_python() {
    use std::os::unix::fs::PermissionsExt;

    let site = tempfile::tempdir().unwrap();
    fs_err::write(
        site.path().join("sitecustomize.py"),
        "print('polluted by sitecustomize')\n",
    )
    .unwrap();
    fs_err::write(
        site.path().join("json.py"),
        "raise RuntimeError('shadowed json')\n",
    )
    .unwrap();
    let wrapper = site.path().join("python-wrapper");
    fs_err::write(
        &wrapper,
        r#"#!/usr/bin/env bash
if [[ "$1" != "-I" || "$2" != "-S" ]]; then
    echo "expected -I -S, got: $*" >&2
    exit 64
fi
exec "$SERC_REAL_PYTHON" "$@"
"#,
    )
    .unwrap();
    fs_err::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();

    let resolver = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../scripts/benchmarks/resolve-cargo-artifact.sh");
    let python = std::env::var("SERC_TEST_PYTHON").unwrap_or_else(|_| "python3".to_string());
    let mut child = Command::new("bash")
        .arg(resolver)
        .arg(wrapper)
        .env("SERC_REAL_PYTHON", python)
        .env("PYTHONPATH", site.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"{\"reason\":\"compiler-artifact\",\"target\":{\"name\":\"serc\",\"kind\":[\"bin\"]},\"executable\":\"/tmp/target/release/serc\"}\n")
        .unwrap();
    let output = child.wait_with_output().unwrap();

    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, b"/tmp/target/release/serc\n");
    assert!(output.stderr.is_empty(), "{output:?}");
}
