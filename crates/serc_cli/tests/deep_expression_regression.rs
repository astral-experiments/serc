#[cfg(unix)]
#[allow(dead_code)]
mod common;

#[cfg(unix)]
use std::process::Command;

#[cfg(unix)]
#[test]
#[ignore = "Serc currently aborts on a valid deep expression; run with --release"]
fn valid_expression_does_not_abort_the_compiler() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("deep.py");
    let other = directory.path().join("other.py");
    let expression = std::iter::repeat_n("x", 20_000)
        .collect::<Vec<_>>()
        .join("+");
    std::fs::write(&source, format!("x={expression}\n")).unwrap();
    std::fs::write(&other, "x=1\n").unwrap();

    let native = common::python_3145_command()
        .args(["-m", "py_compile"])
        .arg(&source)
        .output()
        .unwrap();
    assert!(native.status.success(), "CPython: {native:?}");

    let mut results = Vec::new();
    for jobs in ["1", "2"] {
        // Fix the main-thread stack at the normal Linux default and suppress core dumps.
        // Rayon uses its own 8 MiB stacks, including for the two-file parallel case.
        let mut command = Command::new("sh");
        command
            .args(["-c", "ulimit -c 0; ulimit -s 8192; exec \"$@\"", "--"])
            .arg(env!("CARGO_BIN_EXE_serc"))
            .args(["compile", "--check", "--quiet", "--jobs", jobs])
            .arg(&source);
        if jobs == "2" {
            command.arg(&other);
        }
        results.push(command.output().unwrap());
    }
    assert!(
        results.iter().all(|result| result.status.success()),
        "serial/parallel: {results:?}"
    );
}
