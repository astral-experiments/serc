#[allow(dead_code)]
mod common;

#[test]
fn expression_files_compile_serially_and_in_parallel() {
    use std::process::Command;

    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("expressions.py");
    let other = directory.path().join("annotations.py");
    std::fs::write(
        &source,
        "def f(left, right, flag):\n    return (left + right) if flag else None\n",
    )
    .unwrap();
    std::fs::write(
        &other,
        "from __future__ import annotations\nvalue: Resource.Type | None\n",
    )
    .unwrap();

    let native = common::python_3145_command()
        .args(["-m", "py_compile"])
        .arg(&source)
        .arg(&other)
        .output()
        .unwrap();
    assert!(native.status.success(), "CPython: {native:?}");

    for jobs in ["1", "2"] {
        let result = Command::new(env!("CARGO_BIN_EXE_serc"))
            .args(["compile", "--check", "--quiet", "--jobs", jobs])
            .arg(&source)
            .arg(&other)
            .output()
            .unwrap();
        assert!(result.status.success(), "jobs={jobs}: {result:?}");
    }
}
