mod common;

use std::fmt::Write as _;
use std::process::Command;

use serc::{
    CompileError, CompileOptions, PythonVersion, compile_bytes_with_options, compile_with_options,
};

fn nested_source(levels: usize, indent: &str) -> String {
    let mut source = String::new();
    for level in 0..levels {
        writeln!(source, "{}if True:", indent.repeat(level)).unwrap();
    }
    writeln!(source, "{}pass", indent.repeat(levels)).unwrap();
    source
}

#[test]
fn indentation_depth_matches_cpython() {
    // Match the CLI's stack budget for the accepted 99-level controls. General
    // recursive compiler stack usage is tracked separately from this syntax limit.
    std::thread::Builder::new()
        .stack_size(8 * 1024 * 1024)
        .spawn(check_indentation_depth)
        .unwrap()
        .join()
        .unwrap();
}

fn check_indentation_depth() {
    let boundary = nested_source(99, " ");
    let cases = [
        (boundary.clone(), true),
        (nested_source(99, "\t"), true),
        (nested_source(100, " "), false),
        (nested_source(100, "\t"), false),
        // Dedenting empties the stack: this is a depth limit, not a total count.
        (format!("{boundary}{boundary}"), true),
        // Physical indentation in comments, strings and continuations is not a level.
        (
            format!("{boundary}{}# comment\npass\n", " ".repeat(100)),
            true,
        ),
        (
            format!("text = '''\n{}'''\n", nested_source(100, " ")),
            true,
        ),
        (format!("value = (\n{}1\n)\n", " ".repeat(100)), true),
        // A single large indentation is one level, and a form feed resets its columns.
        (nested_source(1, &" ".repeat(100)), true),
        (format!("{boundary}\x0cpass\n"), true),
    ];
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("indentation.py");
    let oracle = r"
import marshal, sys
try:
    code = compile(sys.argv[1], sys.argv[2], 'exec', dont_inherit=True, optimize=0)
except IndentationError as error:
    print(error.msg)
    raise SystemExit(1)
sys.stdout.buffer.write(marshal.dumps(code))
";
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        let options = CompileOptions {
            python_version: version,
            ..CompileOptions::default()
        };
        for (source, accepted) in &cases {
            let mut python = match version {
                PythonVersion::Py312 => common::python_312_command(),
                PythonVersion::Py313 => common::python_313_command(),
                PythonVersion::Py314 => common::python_3145_command(),
                PythonVersion::Py315 => common::python_315_command(),
            };
            let native = python
                .args(["-c", oracle, source])
                .arg(&path)
                .output()
                .unwrap();
            assert_eq!(native.status.success(), *accepted, "{version}: {native:?}");
            for compiled in [
                compile_with_options(source, path.to_str().unwrap(), &options),
                compile_bytes_with_options(source.as_bytes(), path.to_str().unwrap(), &options),
            ] {
                if *accepted {
                    assert_eq!(compiled.unwrap().marshal(), native.stdout, "{version}");
                } else {
                    assert_eq!(
                        String::from_utf8_lossy(&native.stdout).trim(),
                        "too many levels of indentation"
                    );
                    assert!(
                        compiled
                            .unwrap_err()
                            .to_string()
                            .contains("too many levels of indentation")
                    );
                }
            }
            fs_err::write(&path, source).unwrap();
            let output = Command::new(env!("CARGO_BIN_EXE_serc"))
                .args([
                    "compile",
                    "--check",
                    "--quiet",
                    "--python-version",
                    &version.to_string(),
                ])
                .arg(&path)
                .output()
                .unwrap();
            assert_eq!(output.status.success(), *accepted, "{version}: {output:?}");
            if !accepted {
                assert!(
                    String::from_utf8_lossy(&output.stderr)
                        .contains("too many levels of indentation")
                );
            }
        }
    }
}

#[test]
fn indentation_depth_respects_earlier_errors() {
    std::thread::Builder::new()
        .stack_size(8 * 1024 * 1024)
        .spawn(check_indentation_error_order)
        .unwrap()
        .join()
        .unwrap();
}

fn check_indentation_error_order() {
    let deep = nested_source(100, " ");
    let indentation_start = deep.find(&" ".repeat(100)).unwrap();
    let delimiters = format!("value = {}1{}\n", "(".repeat(201), ")".repeat(201));
    let cases = [
        (
            format!("{deep}bad =\n"),
            "too many levels of indentation",
            indentation_start,
        ),
        (
            format!("{deep}{delimiters}"),
            "too many levels of indentation",
            indentation_start,
        ),
        (
            format!("{deep}value = 0b2\n"),
            "too many levels of indentation",
            indentation_start,
        ),
        (
            format!("{deep})\n"),
            "too many levels of indentation",
            indentation_start,
        ),
        (format!("bad =\n{deep}"), "invalid syntax", 5),
        (
            format!("bad =\n{deep}{delimiters}"),
            "invalid syntax",
            5,
        ),
        (
            format!("{delimiters}{deep}"),
            "too many nested parentheses",
            "value = ".len() + 200,
        ),
    ];
    let oracle = r"
import sys
source = sys.argv[1]
if sys.argv[2] == 'bytes':
    source = source.encode('utf-8')
try:
    compile(source, 'indentation.py', 'exec', dont_inherit=True, optimize=0)
except SyntaxError as error:
    print(error.msg)
else:
    raise SystemExit('expected a syntax error')
";
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        let options = CompileOptions {
            python_version: version,
            ..CompileOptions::default()
        };
        for (source, expected_message, expected_start) in &cases {
            for mode in ["text", "bytes"] {
                let mut python = match version {
                    PythonVersion::Py312 => common::python_312_command(),
                    PythonVersion::Py313 => common::python_313_command(),
                    PythonVersion::Py314 => common::python_3145_command(),
                    PythonVersion::Py315 => common::python_315_command(),
                };
                let native = python.args(["-c", oracle, source, mode]).output().unwrap();
                assert!(native.status.success(), "{version} {mode}: {native:?}");
                assert_eq!(String::from_utf8_lossy(&native.stdout).trim(), *expected_message);
                let result = if mode == "bytes" {
                    compile_bytes_with_options(source.as_bytes(), "indentation.py", &options)
                } else {
                    compile_with_options(source, "indentation.py", &options)
                };
                let error = result.unwrap_err();
                assert_eq!(error.range().unwrap().start().to_usize(), *expected_start);
                match error {
                    CompileError::InvalidSyntax { message, .. } => {
                        assert_eq!(message, *expected_message, "{version} {mode}");
                    }
                    CompileError::Parse(_) => {
                        assert_eq!(*expected_message, "invalid syntax", "{version} {mode}");
                    }
                    error => panic!("{version} {mode}: unexpected error {error:?}"),
                }
            }
        }
    }
}
