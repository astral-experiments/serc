mod common;

use serc::{
    CompileError, CompileOptions, PythonVersion, compile_bytes_with_options, compile_with_options,
};

fn byte_offset(source: &str, line: usize, column: usize) -> usize {
    let prefix: usize = source
        .split_inclusive('\n')
        .take(line - 1)
        .map(str::len)
        .sum();
    let line = source[prefix..].split('\n').next().unwrap();
    prefix
        + line
            .char_indices()
            .nth(column - 1)
            .map_or(line.len(), |(offset, _)| offset)
}

#[test]
fn attribute_named_expression_diagnostic_matches_python() {
    let sources = [
        "value = (obj.attr := 0)\n",
        "(obj.attr.more := 0)\n",
        "(factory().attr := 0)\n",
        "(obj[0].attr := 0)\n",
        "(((obj.attr)) := 0)\n",
        "((obj).attr := 0)\n",
        "(obj . attr := 0)\n",
        "(obj[α + 1].β := 1)\n",
        "(obj.\n attr := 1)\n",
        "(obj.\r\n attr := 1)\r\n",
        "(1 .real := 0)\n",
        "f'{(obj.attr := 0)}'\n",
        "f'{(obj.attr := 0)=}'\n",
        "class C:\n value = (obj.attr := 0)\n",
        "def f():\n return (obj.attr := 0)\n",
    ];
    let oracle = r"
import sys
expected = None
for source in (sys.argv[1], sys.argv[1].encode()):
    try:
        compile(source, 'case.py', 'exec', dont_inherit=True)
    except SyntaxError as error:
        actual = (error.msg, error.lineno, error.offset, error.end_lineno, error.end_offset)
        assert expected is None or expected == actual
        expected = actual
    else:
        raise AssertionError('expected a syntax error')
print(expected[0])
print(*expected[1:])
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
        for source in sources {
            let mut python = match version {
                PythonVersion::Py312 => common::python_312_command(),
                PythonVersion::Py313 => common::python_313_command(),
                PythonVersion::Py314 => common::python_3145_command(),
                PythonVersion::Py315 => common::python_315_command(),
            };
            let output = python.args(["-c", oracle, source]).output().unwrap();
            assert!(output.status.success(), "{version}: {source:?}: {output:?}");
            let output = String::from_utf8(output.stdout).unwrap();
            let mut lines = output.lines();
            let message = lines.next().unwrap();
            let positions: Vec<usize> = lines
                .next()
                .unwrap()
                .split_whitespace()
                .map(|value| value.parse().unwrap())
                .collect();
            let normalized = source.replace("\r\n", "\n");
            let expected = (
                byte_offset(&normalized, positions[0], positions[1]),
                byte_offset(&normalized, positions[2], positions[3]),
            );
            for result in [
                compile_with_options(source, "case.py", &options),
                compile_bytes_with_options(source.as_bytes(), "case.py", &options),
            ] {
                let error = result.unwrap_err();
                assert!(
                    error.to_string().ends_with(message),
                    "{version}: {source:?}: {message:?} != {error}"
                );
                let range = error.range().unwrap();
                assert_eq!(
                    (range.start().to_usize(), range.end().to_usize()),
                    expected,
                    "{version}: {source:?}: {error}"
                );
            }
        }
    }
}

#[test]
fn other_named_expression_errors_are_preserved() {
    for source in [
        "(obj[0] := 0)\n",
        "(obj.attr :=)\n",
        "(obj.attr := 0\n",
        "(obj.attr := 0)\nx = 'unterminated\n",
        "(obj.attr := 0)\n)\n",
        "@\n(obj.attr := 0)\n",
        "(obj.attr := (other.x := 3))\n",
    ] {
        let error =
            compile_with_options(source, "case.py", &CompileOptions::default()).unwrap_err();
        assert!(
            matches!(error, CompileError::Parse(_)),
            "{source:?}: {error}"
        );
    }
    for source in [
        "(obj := 0)\n",
        "f'{obj.attr := 0}'\n",
        "value = '(obj.attr := 0)'\n",
        "# (obj.attr := 0)\npass\n",
    ] {
        assert!(compile_with_options(source, "case.py", &CompileOptions::default()).is_ok());
    }
}

#[test]
fn deep_attribute_target_does_not_overflow_visitor_stack() {
    let source = format!("(obj{} := 0)\n", ".field".repeat(2000));
    let error = compile_with_options(&source, "case.py", &CompileOptions::default()).unwrap_err();
    assert!(
        error
            .to_string()
            .ends_with("cannot use assignment expressions with attribute"),
        "{error}"
    );
}

#[test]
fn unknown_decimal_parse_limit_takes_precedence() {
    let source = format!(
        "match value:\n    case _:\n        value = (obj.attr := 0)\n        integer = {}\n",
        "9".repeat(641)
    );
    let error = compile_with_options(&source, "case.py", &CompileOptions::default()).unwrap_err();
    assert!(
        matches!(
            error,
            CompileError::Unsupported(ref message) if message == "decimal integer parsing limit is not known"
        ),
        "{error}"
    );
}
