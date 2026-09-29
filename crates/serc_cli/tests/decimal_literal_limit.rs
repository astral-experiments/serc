mod common;

use std::process::Command;

use serc::{CompileError, CompileOptions, compile_bytes_with_options, compile_with_options};

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
    for (unsupported, message) in [
        (template, "Cannot use t-strings"),
        (default, "Cannot set default type for a type parameter"),
        (lazy, "Cannot use `lazy` import statement"),
        (unpack, "Cannot use iterable unpacking"),
        (exception, "Multiple exception types must be parenthesized"),
        (pattern, "Unary '+' is not allowed in a literal pattern"),
    ] {
        cases.push((format!("{unsupported}{large}"), message));
        cases.push((format!("{large}{unsupported}"), message));
    }
    cases.extend([
        (format!("t'{{{digits}}}'\n"), "Cannot use t-strings"),
        (
            format!("def f[T = {digits}]():\n    pass\n"),
            "Cannot set default type for a type parameter",
        ),
        (
            format!("def f[T: {digits} = int]():\n    pass\n"),
            "Cannot set default type for a type parameter",
        ),
        (
            format!("value = [*{digits} for v in ()]\n"),
            "Cannot use iterable unpacking",
        ),
        (
            format!("value = [*v for v in [{digits}]]\n"),
            "Cannot use iterable unpacking",
        ),
        (
            format!("value = [*v for v in () if {digits}]\n"),
            "Cannot use iterable unpacking",
        ),
        (
            format!("value = [*v for v in ()] + {digits}\n"),
            "Cannot use iterable unpacking",
        ),
        (
            format!("f([*v for v in ()], {digits})\n"),
            "Cannot use iterable unpacking",
        ),
        (
            format!("value = [[*v for v in ()] for v in [{digits}]]\n"),
            "Cannot use iterable unpacking",
        ),
        (
            format!("value = {{*v for v in [{digits}]}}\n"),
            "Cannot use iterable unpacking",
        ),
        (
            format!("value = {{**v for v in [{digits}]}}\n"),
            "Cannot use dictionary unpacking",
        ),
        (
            format!("value = (*v for v in [{digits}])\n"),
            "Cannot use iterable unpacking",
        ),
        (
            format!("try:\n    pass\nexcept TypeError, {digits}:\n    pass\n"),
            "Multiple exception types must be parenthesized",
        ),
        (
            format!("try:\n    pass\nexcept TypeError, ValueError:\n    value = {digits}\n"),
            "Multiple exception types must be parenthesized",
        ),
        (
            format!("match 1:\n    case +{digits}:\n        pass\n"),
            "Unary '+' is not allowed in a literal pattern",
        ),
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
    for version in common::PYTHON_VERSIONS {
        let options = CompileOptions {
            python_version: version,
            ..CompileOptions::default()
        };
        for (source, unsupported_message) in &cases {
            let mut python = common::python_command(version);
            let native = python.args(["-c", oracle, source]).output().unwrap();
            assert!(native.status.success(), "{version}: {native:?}");
            let expected = String::from_utf8(native.stdout).unwrap();
            assert!(matches!(expected.trim(), "decimal" | "other"), "{version}");
            let error = compile_with_options(source, "ordering.py", &options).unwrap_err();
            let CompileError::InvalidSyntax { message, range } = error else {
                panic!("{version}: expected a syntax diagnostic, got {error:?}");
            };
            let expected_message = if expected.trim() == "decimal" {
                "Exceeds the limit (4300 digits)"
            } else {
                unsupported_message
            };
            assert!(message.contains(expected_message), "{version}: {message}");
            assert!(!range.is_empty(), "{version}: missing diagnostic range");
        }
    }
}

#[test]
fn configured_decimal_literal_limit_matches_cpython() {
    let cases = [
        (640, format!("value = {}\n", "1".repeat(640)), true),
        (640, format!("value = {}\n", "1".repeat(641)), false),
        (641, format!("value = {}\n", "1".repeat(641)), true),
        (640, format!("value = {}1\n", "1_".repeat(639)), true),
        (640, format!("value = {}1\n", "1_".repeat(640)), false),
        (0, format!("value = {}\n", "1".repeat(4301)), true),
        (4300, format!("value = {}\n", "1".repeat(4301)), false),
        (5000, format!("value = {}\n", "1".repeat(5000)), true),
        (5000, format!("value = {}\n", "1".repeat(5001)), false),
        (
            640,
            format!("match x:\n case _: pass\ny = {}\n", "1".repeat(641)),
            false,
        ),
        (
            640,
            format!("if False:\n value = {}\n", "1".repeat(641)),
            false,
        ),
        (
            640,
            format!(
                "zeros = {}\nhexadecimal = 0x{}\noctal = 0o{}\nbinary = 0b{}\n",
                "0".repeat(641),
                "f".repeat(641),
                "7".repeat(641),
                "1".repeat(641),
            ),
            true,
        ),
        (i32::MAX as u32, "value = 1\n".to_owned(), true),
    ];
    let path = std::path::Path::new("configured_decimal.py");
    let oracle = r"
import marshal, sys
sys.set_int_max_str_digits(int(sys.argv[1]))
try:
    code = compile(sys.argv[2], sys.argv[3], 'exec', dont_inherit=True, optimize=0)
except SyntaxError as error:
    print(error.msg)
    raise SystemExit(1)
sys.stdout.buffer.write(marshal.dumps(code))
";
    for version in common::PYTHON_VERSIONS {
        for (limit, source, accepted) in &cases {
            let options = CompileOptions {
                python_version: version,
                ..CompileOptions::default()
            };
            let mut python = common::python_command(version);
            let native = python
                .args(["-c", oracle, &limit.to_string(), source])
                .arg(path)
                .output()
                .unwrap();
            assert_eq!(native.status.success(), *accepted, "{version}: {native:?}");
            for compiled in [
                serc::compile_with_options_and_integer_limits(
                    source,
                    path.to_str().unwrap(),
                    &options,
                    *limit,
                    None,
                ),
                serc::compile_bytes_with_options_and_integer_limits(
                    source.as_bytes(),
                    path.to_str().unwrap(),
                    &options,
                    *limit,
                    None,
                ),
                serc::compile_bytes_with_path_options_and_integer_limits(
                    source.as_bytes(),
                    path,
                    &options,
                    *limit,
                    None,
                ),
            ] {
                if *accepted {
                    assert_eq!(
                        compiled.unwrap().marshal(),
                        native.stdout,
                        "{version}, {limit}"
                    );
                } else {
                    let error = compiled.unwrap_err();
                    assert!(matches!(error, CompileError::InvalidSyntax { .. }));
                    assert!(
                        error
                            .to_string()
                            .ends_with(String::from_utf8_lossy(&native.stdout).trim()),
                        "{version}, {limit}: {error}"
                    );
                }
            }
        }
    }
}

#[test]
fn decimal_parse_limit_is_validated_and_defaults_are_preserved() {
    for limit in [1, 639, i32::MAX as u32 + 1, u32::MAX] {
        let options = CompileOptions::default();
        for compiled in [
            serc::compile_with_options_and_integer_limits(
                "pass\n", "limit.py", &options, limit, None,
            ),
            serc::compile_bytes_with_options_and_integer_limits(
                b"pass\n", "limit.py", &options, limit, None,
            ),
            serc::compile_bytes_with_path_options_and_integer_limits(
                b"pass\n",
                "limit.py".as_ref(),
                &options,
                limit,
                None,
            ),
        ] {
            assert!(matches!(compiled, Err(CompileError::Unsupported(_))));
        }
    }
    let default = CompileOptions::default();
    for digits in [641, 4300, 4301] {
        let source = format!("value = {}\n", "1".repeat(digits));
        assert_eq!(
            compile_with_options(&source, "limit.py", &default).map(|code| code.marshal()),
            serc::compile_with_options_and_integer_limits(
                &source, "limit.py", &default, 4300, None
            )
            .map(|code| code.marshal()),
        );
    }
}

#[test]
fn decimal_parsing_and_mapping_key_formatting_have_separate_limits() {
    let key = "1".repeat(641);
    let source = format!("match subject:\n case {{{key}: _, {key}: _}}: pass\n");
    for version in common::PYTHON_VERSIONS {
        for (parse_limit, format_limit) in [(640, 0), (0, 640), (4300, 0)] {
            let options = CompileOptions {
                python_version: version,
                ..CompileOptions::default()
            };
            for compiled in [
                serc::compile_with_options_and_integer_limits(
                    &source,
                    "mapping.py",
                    &options,
                    parse_limit,
                    Some(format_limit),
                ),
                serc::compile_bytes_with_options_and_integer_limits(
                    source.as_bytes(),
                    "mapping.py",
                    &options,
                    parse_limit,
                    Some(format_limit),
                ),
                serc::compile_bytes_with_path_options_and_integer_limits(
                    source.as_bytes(),
                    "mapping.py".as_ref(),
                    &options,
                    parse_limit,
                    Some(format_limit),
                ),
            ] {
                let error = compiled.unwrap_err();
                match (parse_limit, format_limit) {
                    (640, 0) => assert!(matches!(error, CompileError::InvalidSyntax { .. })),
                    (0, 640) => assert_eq!(
                        error,
                        CompileError::IntegerStringConversionLimit { max_digits: 640 },
                    ),
                    (4300, 0) => assert!(matches!(error, CompileError::InvalidPattern { .. })),
                    _ => unreachable!(),
                }
            }
        }
    }
}

#[test]
fn explicit_decimal_parse_limit_precedes_later_semantic_validation() {
    let digits = "1".repeat(641);
    let source = format!("match subject:\n case {{1: _, 1: _}}: pass\nlater = {digits}\n");
    let malformed = format!("# {digits}\nmatch subject:\n case +: pass\n");
    for version in common::PYTHON_VERSIONS {
        for limit in [640, 0, 4300] {
            let options = CompileOptions {
                python_version: version,
                ..CompileOptions::default()
            };
            let error = serc::compile_with_options_and_integer_limits(
                &source,
                "ordering.py",
                &options,
                limit,
                None,
            )
            .unwrap_err();
            if limit == 640 {
                assert!(matches!(error, CompileError::InvalidSyntax { .. }));
                assert!(error.to_string().contains("Exceeds the limit (640 digits)"));
            } else {
                assert!(matches!(error, CompileError::InvalidPattern { .. }));
            }
            assert!(matches!(
                serc::compile_with_options_and_integer_limits(
                    &malformed,
                    "ordering.py",
                    &options,
                    limit,
                    None
                ),
                Err(CompileError::Parse(_) | CompileError::InvalidSyntax { .. })
            ));
        }
    }
}
