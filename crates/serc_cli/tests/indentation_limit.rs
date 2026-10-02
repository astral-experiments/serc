mod common;

use std::fmt::Write as _;

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

#[test]
fn nested_suites_grow_a_small_stack_without_changing_output() {
    let filename = "small_stack_suites.py";
    let oracle = "import marshal, sys; code = compile(sys.argv[1], sys.argv[2], 'exec', dont_inherit=True, optimize=0); sys.stdout.buffer.write(marshal.dumps(code))";
    for condition in ["True", "False", "flag"] {
        let source = nested_source(99, " ").replace("if True:", &format!("if {condition}:"));
        for version in common::PYTHON_VERSIONS {
            let native = common::python_command(version)
                .args(["-c", oracle, &source, filename])
                .output()
                .unwrap();
            assert!(native.status.success(), "{version} {condition}: {native:?}");
            let source = source.clone();
            std::thread::Builder::new()
                .name(format!("suites-{version}-{condition}"))
                .stack_size(512 * 1024)
                .spawn(move || {
                    let options = CompileOptions {
                        python_version: version,
                        ..CompileOptions::default()
                    };
                    for compiled in [
                        compile_with_options(&source, filename, &options),
                        compile_bytes_with_options(source.as_bytes(), filename, &options),
                    ] {
                        assert_eq!(
                            compiled.unwrap().marshal(),
                            native.stdout,
                            "{version} {condition}",
                        );
                    }
                })
                .unwrap()
                .join()
                .unwrap();
        }
    }
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
    for version in common::PYTHON_VERSIONS {
        let options = CompileOptions {
            python_version: version,
            ..CompileOptions::default()
        };
        for (source, accepted) in &cases {
            let mut python = common::python_command(version);
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
            let output = common::check(&path, version);
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
    let deep_delimiters = format!("{}{delimiters}", deep.strip_suffix("pass\n").unwrap());
    let early_yield = "def f():\n    yield, x\n";
    let yield_start = early_yield.find(',').unwrap();
    let interpolated_yield = "def f():\n    x = f\"{yield, x}\"\n";
    let mut cases = vec![
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
        (format!("bad =\n{deep}{delimiters}"), "invalid syntax", 5),
        (
            format!("{delimiters}{deep}"),
            "too many nested parentheses",
            "value = ".len() + 200,
        ),
        (
            format!("{early_yield}{deep}"),
            "invalid syntax",
            yield_start,
        ),
        (
            format!("{deep}{early_yield}"),
            "too many levels of indentation",
            indentation_start,
        ),
        (
            format!("{early_yield}if : pass\n{deep}"),
            "invalid syntax",
            yield_start,
        ),
        (
            format!("if : pass\n{early_yield}{deep}"),
            "invalid syntax",
            "if ".len(),
        ),
        (
            format!("{early_yield}{deep}{delimiters}"),
            "invalid syntax",
            yield_start,
        ),
        (
            format!("{early_yield}{delimiters}{deep}"),
            "too many nested parentheses",
            early_yield.len() + "value = ".len() + 200,
        ),
        (
            format!("{early_yield}value = (1)\n{deep}"),
            "invalid syntax",
            yield_start,
        ),
        (
            format!("{early_yield}value = 1 + \\\n 2\n{deep}"),
            "invalid syntax",
            yield_start,
        ),
        (
            format!("{early_yield}{deep})\n"),
            "invalid syntax",
            yield_start,
        ),
        (
            format!("{early_yield}{deep}value = 'unterminated\n"),
            "invalid syntax",
            yield_start,
        ),
        (
            format!("{interpolated_yield}{deep}"),
            "f-string: expecting '=', or '!', or ':', or '}'",
            interpolated_yield.find(',').unwrap(),
        ),
    ];
    for token in ["?", "$", "`"] {
        cases.push((
            format!("value = {token}\n{deep_delimiters}"),
            "invalid syntax",
            "value = ".len(),
        ));
        cases.push((
            format!("{deep_delimiters}value = {token}\n"),
            "too many levels of indentation",
            indentation_start,
        ));
    }
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
    for version in common::PYTHON_VERSIONS {
        let options = CompileOptions {
            python_version: version,
            ..CompileOptions::default()
        };
        for (source, expected_message, expected_start) in &cases {
            for mode in ["text", "bytes"] {
                let mut python = common::python_command(version);
                let native = python.args(["-c", oracle, source, mode]).output().unwrap();
                assert!(native.status.success(), "{version} {mode}: {native:?}");
                assert_eq!(
                    String::from_utf8_lossy(&native.stdout).trim(),
                    *expected_message
                );
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

#[test]
fn indentation_depth_preserves_earlier_legacy_template_error() {
    let deep = nested_source(100, " ");
    let template = "value = t'hello'\n";
    let delimiters = format!("value = {}1{}\n", "(".repeat(201), ")".repeat(201));
    for version in [PythonVersion::Py312, PythonVersion::Py313] {
        let options = CompileOptions {
            python_version: version,
            ..CompileOptions::default()
        };
        for source in [
            format!("{template}{deep}"),
            format!("{template}def f():\n    yield, x\n{deep}"),
            format!("{template}{deep}{delimiters}"),
        ] {
            for result in [
                compile_with_options(&source, "indentation.py", &options),
                compile_bytes_with_options(source.as_bytes(), "indentation.py", &options),
            ] {
                let error = result.unwrap_err();
                assert!(
                    error.to_string().contains("Cannot use t-strings"),
                    "{version}: {error}"
                );
                assert!(error.range().unwrap().start().to_usize() < template.len());
            }
        }
    }
}

#[test]
fn indentation_depth_preserves_earlier_version_errors() {
    let deep = nested_source(100, " ");
    let template = "value = t'hello'\n";
    let delimiters = format!("value = {}1{}\n", "(".repeat(201), ")".repeat(201));
    for (version, prefix, expected) in [
        (
            PythonVersion::Py312,
            "type A[T = int] = T\n",
            "Cannot set default type",
        ),
        (
            PythonVersion::Py312,
            "lazy import os\n",
            "Cannot use `lazy` import",
        ),
        (
            PythonVersion::Py313,
            "lazy import os\n",
            "Cannot use `lazy` import",
        ),
        (
            PythonVersion::Py314,
            "lazy import os\n",
            "Cannot use `lazy` import",
        ),
    ] {
        let options = CompileOptions {
            python_version: version,
            ..CompileOptions::default()
        };
        for middle in ["", template, "def f():\n    yield, x\n"] {
            for ending in ["", delimiters.as_str()] {
                let source = format!("{prefix}{middle}{deep}{ending}");
                for result in [
                    compile_with_options(&source, "indentation.py", &options),
                    compile_bytes_with_options(source.as_bytes(), "indentation.py", &options),
                ] {
                    let error = result.unwrap_err();
                    assert!(error.to_string().contains(expected), "{version}: {error}");
                    assert!(error.range().unwrap().start().to_usize() < prefix.len());
                }
            }
        }
    }
}
