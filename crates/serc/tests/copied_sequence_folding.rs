use std::io::Write as _;
use std::process::{Command, Stdio};

use serc::{CompileOptions, PythonVersion, compile_bytes_with_options, compile_with_options};

fn assert_matches_cpython_version(source: &str, version: PythonVersion) {
    let target = version.target();
    let (variable, fallback) = match version {
        PythonVersion::Py312 => ("SERC_TEST_PYTHON_312", "python3.12"),
        PythonVersion::Py313 => ("SERC_TEST_PYTHON_313", "python3.13"),
        PythonVersion::Py314 => ("SERC_TEST_PYTHON", "python3.14"),
        PythonVersion::Py315 => ("SERC_TEST_PYTHON_315", "python3.15"),
    };
    let python = std::env::var_os(variable).unwrap_or_else(|| fallback.into());
    let oracle = r"
import importlib.util, marshal, sys
assert sys.implementation.name == 'cpython'
assert sys.version.split()[0] == sys.argv[1]
assert importlib.util.MAGIC_NUMBER.hex() == sys.argv[2]
code = compile(sys.stdin.buffer.read(), sys.argv[3], 'exec', dont_inherit=True, optimize=0)
sys.stdout.buffer.write(marshal.dumps(code))
";
    let filename = "copied_sequence_folding.py";
    let mut child = Command::new(python)
        .args(["-B", "-I", "-S", "-X", "utf8", "-c", oracle])
        .args([
            target.tag.trim_start_matches('v'),
            &target.magic_hex(),
            filename,
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(source.as_bytes())
        .unwrap();
    let expected = child.wait_with_output().unwrap();
    assert!(
        expected.status.success(),
        "CPython oracle failed: {}",
        String::from_utf8_lossy(&expected.stderr)
    );
    let options = CompileOptions {
        python_version: version,
        ..CompileOptions::default()
    };
    for actual in [
        compile_with_options(source, filename, &options).unwrap(),
        compile_bytes_with_options(source.as_bytes(), filename, &options).unwrap(),
    ] {
        assert_eq!(actual.marshal(), expected.stdout, "{version:?}: {source}");
    }
}

#[test]
fn folds_sequence_operations_only_after_exit_copying() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for (left, right) in [
            ("'first'", "'second'"),
            ("b'first'", "b'second'"),
            ("(1, 2)", "(3, 4)"),
            ("'\\ud800'", "'text'"),
        ] {
            for expression in [
                format!("({left} if flag else {right}) + {left}"),
                format!("({left} if flag else {right}) * 4"),
                format!("4 * ({left} if flag else {right})"),
                format!("({left} if flag else {right}) + {left} + {right}"),
                format!("({left} if flag else {right}) * 4097"),
                format!("({left} if flag else {right}) * False"),
                format!("({left} if flag else {right}) * -1"),
            ] {
                let source = format!("def f(flag):\n    return {expression}\n");
                assert_matches_cpython_version(&source, version);
            }
        }
    }
}

#[test]
fn copied_sequence_folds_keep_locations_and_operand_admission_order() {
    for version in [PythonVersion::Py314, PythonVersion::Py315] {
        for (left, right) in [
            ("'first'", "'second'"),
            ("b'first'", "b'second'"),
            ("(1, 2)", "(3, 4)"),
        ] {
            for expression in [
                format!("(\n        ({left} if flag else {right})\n        + {left}\n    )"),
                format!("(\n        ({left} if flag else {right}) *\n        4\n    )"),
                format!("(({left} if flag else {right}) + {left},)"),
            ] {
                let source = format!(
                    "def f(flag, early):\n    if early:\n        return {expression}\n    return {left}, {right}\n"
                );
                assert_matches_cpython_version(&source, version);
            }
        }
    }
}

#[test]
fn copied_empty_bytes_folds_preserve_first_cache_admission() {
    for version in [PythonVersion::Py314, PythonVersion::Py315] {
        for prefix in ["", "empty = b''\n", "def earlier():\n    return None\n"] {
            for expression in [
                "(b'first' if flag else b'second') * False",
                "(b'' if flag else b'first') * -1",
                "(b'first' if flag else value) * 0",
            ] {
                let source = format!(
                    "{prefix}def f(flag, value):\n    return {expression}\n\ndef later():\n    return b''\n"
                );
                assert_matches_cpython_version(&source, version);
            }
        }
    }
}
