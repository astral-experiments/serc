mod common;

use serc::{
    CompileError, CompileOptions, PythonVersion, compile_bytes_with_options, compile_with_options,
};

#[test]
fn unknown_decimal_parse_limit_takes_precedence() {
    let source = format!(
        "match value:\n    case _:\n        def f():\n            yield from a, b\n        integer = {}\n",
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
fn yield_from_tuple_diagnostics_match_native() {
    let deep = format!("def f():\n    yield from {}a, b\n", "+".repeat(300));
    let many_stars = format!("def f():\n    yield from {}\n", vec!["*a"; 1000].join(", "));
    let cases = [
        deep.as_str(),
        many_stars.as_str(),
        "def f():\n    yield from a, b\n",
        "def f():\n    yield from a,\n",
        "def f():\n    yield from *a, b\n",
        "def f():\n    yield from a, *b\n",
        "def f():\n    yield from *a, *b\n",
        "def f():\n    yield from (*a), b\n",
        "def f():\n    yield from ((*a)), b\n",
        "def f():\n    yield from (*a), *b\n",
        "def f():\n    yield from (*a,), b\n",
        "def f():\n    yield from (a), b\n",
        "def f():\n    yield from (a,b), c\n",
        "def f():\n    yield from foo(a,b), c\n",
        "def f():\n    yield from ((foo(a,b))), c\n",
        "def f():\n    yield from a if b else c, d\n",
        "def f():\n    yield from (lambda a,b:a), c\n",
        "def f():\n    yield from (x:=a), b\n",
        "def f():\n    yield from [x for x in a], b\n",
        "def f():\n    yield from f\"{a,b}\", c\n",
        "def f():\n    yield from (f\"{(a,b)}\", x), c\n",
        "def f():\n    yield from (a # comment\n      , b), c\n",
        "def f():\n    yield from α, β\n",
        "def f():\n    yield from a \\\n      , b\n",
        "def f():\n    yield from (a, b), c, d\n",
        "def f():\n    yield from a, (b, c)\n",
        "def f():\n    yield from a \\\n       ,\n",
    ];
    let oracle = r"
import sys
try:
    compile(sys.argv[1], 'case.py', 'exec', dont_inherit=True)
except SyntaxError as error:
    print(error.msg)
    print(error.lineno, error.offset, error.end_lineno, error.end_offset)
else:
    raise AssertionError('expected a syntax error')
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
        for source in cases {
            let mut python = match version {
                PythonVersion::Py312 => common::python_312_command(),
                PythonVersion::Py313 => common::python_313_command(),
                PythonVersion::Py314 => common::python_3145_command(),
                PythonVersion::Py315 => common::python_315_command(),
            };
            let native = python.args(["-c", oracle, source]).output().unwrap();
            assert!(native.status.success(), "{version}: {source:?}: {native:?}");
            let output = String::from_utf8(native.stdout).unwrap();
            let mut lines = output.lines();
            let message = lines.next().unwrap();
            let positions: Vec<usize> = lines
                .next()
                .unwrap()
                .split_whitespace()
                .map(|value| value.parse().unwrap())
                .collect();
            let start = byte_offset(source, positions[0], positions[1]);
            let end = byte_offset(source, positions[2], positions[3]);
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
                    (start, end),
                    "{version}: {source:?}: {error}"
                );
            }
        }
    }
}

#[test]
fn yield_from_tuple_does_not_override_other_parser_errors() {
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
        for source in [
            "def f():\n    raise a, b\n",
            "def f():\n    yield from a, b\n    )\n",
            "def f():\n    yield from a, b\n    x = 123abc\n",
            "def f():\n    yield from a, b\n    x = 'unterminated\n",
        ] {
            let error = compile_with_options(source, "case.py", &options).unwrap_err();
            assert!(
                matches!(error, CompileError::Parse(_)),
                "{version}: {source:?}: {error}"
            );
        }
        for source in [
            "def f():\n    yield from (a, b)\n",
            "def f():\n    yield a, b\n",
            "def f():\n    yield from a # , b\n",
        ] {
            compile_with_options(source, "case.py", &options)
                .unwrap_or_else(|error| panic!("{version}: {source:?}: {error}"));
        }
    }
}
