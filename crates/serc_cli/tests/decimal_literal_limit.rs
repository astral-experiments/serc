mod common;

use std::process::Command;

use serc::{CompileOptions, PythonVersion, compile_bytes_with_options, compile_with_options};

#[test]
fn decimal_literal_digit_limit_matches_cpython() {
    let at_limit = "1".repeat(4300);
    let over_limit = "1".repeat(4301);
    let cases = [
        (format!("value = {at_limit}\n"), true),
        (format!("value = {over_limit}\n"), false),
        (format!("value = -{over_limit}\n"), false),
        (format!("value = {}1\n", "1_".repeat(4300)), false),
        (format!("value = {}1\n", "1_".repeat(4299)), true),
        // The native parser's machine-integer fast path accepts arbitrarily many zeros.
        (format!("value = {}\n", "0".repeat(5000)), true),
        (format!("value = {}0\n", "0_".repeat(5000)), true),
        (format!("value = 0x{}\n", "f".repeat(4301)), true),
        (format!("value = 0o{}\n", "7".repeat(4301)), true),
        (format!("value = 0b{over_limit}\n"), true),
        (format!("value = {over_limit}.0\n"), true),
        (format!("value = {over_limit}j\n"), true),
        (format!("value = '{over_limit}'\n# {over_limit}\n"), true),
        (format!("if False:\n    value = {over_limit}\n"), false),
        (
            format!("def f():\n    return\n    value = {over_limit}\n"),
            false,
        ),
        (format!("value = f'{{{over_limit}}}'\n"), false),
    ];
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("decimal_literal.py");
    let oracle = r"
import marshal, sys
assert sys.get_int_max_str_digits() == 4300
try:
    code = compile(sys.argv[1], sys.argv[2], 'exec', dont_inherit=True, optimize=0)
except SyntaxError as error:
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
                    let expected = String::from_utf8_lossy(&native.stdout);
                    assert!(expected.starts_with("Exceeds the limit (4300 digits)"));
                    let error = compiled.unwrap_err();
                    assert!(
                        error.to_string().ends_with(expected.trim()),
                        "{version}: {error}"
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
                        .contains("Exceeds the limit (4300 digits)")
                );
            }
        }
    }
}

#[test]
fn decimal_limit_respects_earlier_version_errors() {
    let digits = "1".repeat(4301);
    let large = format!("value = {digits}\n");
    let template = "t'hello'\n";
    let default = "def f[T = int]():\n    pass\n";
    let lazy = "lazy import math\n";
    let unpack = "value = [*v for v in ()]\n";
    let exception = "try:\n    pass\nexcept TypeError, ValueError:\n    pass\n";
    let pattern = "match 1:\n    case +1:\n        pass\n";
    let mut cases = Vec::new();
    for unsupported in [template, default, lazy, unpack, exception, pattern] {
        cases.push(format!("{unsupported}{large}"));
        cases.push(format!("{large}{unsupported}"));
    }
    cases.extend([
        format!("t'{{{digits}}}'\n"),
        format!("def f[T = {digits}]():\n    pass\n"),
        format!("def f[T: {digits} = int]():\n    pass\n"),
        format!("value = [*{digits} for v in ()]\n"),
        format!("try:\n    pass\nexcept TypeError, {digits}:\n    pass\n"),
        format!("match 1:\n    case +{digits}:\n        pass\n"),
    ]);
    let oracle = r"
import sys
try:
    compile(sys.argv[1], 'ordering.py', 'exec', dont_inherit=True, optimize=0)
except SyntaxError as error:
    print('decimal' if error.msg.startswith('Exceeds the limit (4300 digits)') else 'other')
else:
    print('accepted')
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
        for source in &cases {
            let mut python = match version {
                PythonVersion::Py312 => common::python_312_command(),
                PythonVersion::Py313 => common::python_313_command(),
                PythonVersion::Py314 => common::python_3145_command(),
                PythonVersion::Py315 => common::python_315_command(),
            };
            let native = python.args(["-c", oracle, source]).output().unwrap();
            assert!(native.status.success(), "{version}: {native:?}");
            let expected = String::from_utf8(native.stdout).unwrap();
            assert!(matches!(expected.trim(), "decimal" | "other"), "{version}");
            let error = compile_with_options(source, "ordering.py", &options).unwrap_err();
            assert_eq!(
                error
                    .to_string()
                    .contains("Exceeds the limit (4300 digits)"),
                expected.trim() == "decimal",
                "{version}: {error}"
            );
        }
    }
}
