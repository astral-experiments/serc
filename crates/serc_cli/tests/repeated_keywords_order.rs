mod common;

use serc::{CompileError, CompileOptions, PythonVersion, compile_with_options};

#[test]
fn repeated_keywords_preserve_native_diagnostic_order() {
    const REPEAT_X: &str = "keyword argument repeated: x";
    const REPEAT_Y: &str = "keyword argument repeated: y";
    const DUPLICATE_TYPE: &str = "duplicate type parameter 'T'";
    const LATE_FUTURE: &str = "from __future__ imports must occur at the beginning of the file";
    const UNKNOWN_FUTURE: &str = "future feature imaginary is not defined";
    const GLOBAL: &str = "name 'x' is assigned to before global declaration";
    let cases = [
        (
            "f(x=1, x=2)\nfrom __future__ import annotations\n",
            (REPEAT_X, "x=2"),
            (LATE_FUTURE, "from __future__ import annotations"),
        ),
        (
            "f(x=1, x=2)\nfrom __future__ import imaginary\n",
            (REPEAT_X, "x=2"),
            (LATE_FUTURE, "from __future__ import imaginary"),
        ),
        (
            "f(x=1, x=2)\nfrom __future__ import braces\n",
            (REPEAT_X, "x=2"),
            (LATE_FUTURE, "from __future__ import braces"),
        ),
        (
            "f(x=1, x=2)\nfrom __future__ import annotations, imaginary\n",
            (REPEAT_X, "x=2"),
            (LATE_FUTURE, "from __future__ import annotations, imaginary"),
        ),
        (
            "from __future__ import imaginary\nf(x=1, x=2)\n",
            (UNKNOWN_FUTURE, "imaginary"),
            (UNKNOWN_FUTURE, "imaginary"),
        ),
        (
            "from __future__ import imaginary\nf(x=1, x=2)\nfrom __future__ import annotations\n",
            (UNKNOWN_FUTURE, "imaginary"),
            (UNKNOWN_FUTURE, "imaginary"),
        ),
        (
            "from __future__ import annotations\nf(x=1, x=2)\nfrom __future__ import imaginary\n",
            (REPEAT_X, "x=2"),
            (LATE_FUTURE, "from __future__ import imaginary"),
        ),
        (
            "f(x=1, x=2)\nclass C[T, T]: pass\n",
            (DUPLICATE_TYPE, "T"),
            (DUPLICATE_TYPE, "T"),
        ),
        (
            "class C[T, T]: pass\nf(x=1, x=2)\n",
            (DUPLICATE_TYPE, "T"),
            (DUPLICATE_TYPE, "T"),
        ),
        (
            "def f[T, T](x=g(y=1, y=2)): pass\n",
            (DUPLICATE_TYPE, "T"),
            (DUPLICATE_TYPE, "T"),
        ),
        (
            "@g(y=1, y=2)\nclass C[T, T]: pass\n",
            (DUPLICATE_TYPE, "T"),
            (DUPLICATE_TYPE, "T"),
        ),
        (
            "f(x=1, x=2)\nx = 1\nglobal x\n",
            (GLOBAL, "global x"),
            (GLOBAL, "global x"),
        ),
        (
            "value: f(x=1, x=2)\ng(y=1, y=2)\n",
            (REPEAT_X, "x=2"),
            (REPEAT_Y, "y=2"),
        ),
        (
            "class C:\n    value: f(x=1, x=2)\n    g(y=1, y=2)\n",
            (REPEAT_X, "x=2"),
            (REPEAT_Y, "y=2"),
        ),
        (
            "if flag:\n    value: f(x=1, x=2)\n    g(y=1, y=2)\n",
            (REPEAT_X, "x=2"),
            (REPEAT_Y, "y=2"),
        ),
        (
            "f(x=1, y=1, y=2, x=2)\n",
            (REPEAT_X, "x=2"),
            (REPEAT_X, "x=2"),
        ),
    ];
    let oracle = r"
import sys
try:
    compile(sys.argv[1], 'synthetic.py', 'exec', dont_inherit=True, optimize=0)
except SyntaxError as error:
    print(error.msg)
    print(error.lineno, error.offset, error.end_lineno, error.end_offset)
    raise SystemExit(1)
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
        let modern = matches!(version, PythonVersion::Py314 | PythonVersion::Py315);
        for (source, legacy, current) in cases {
            let (expected_message, expected_span) = if modern { current } else { legacy };
            let mut python = match version {
                PythonVersion::Py312 => common::python_312_command(),
                PythonVersion::Py313 => common::python_313_command(),
                PythonVersion::Py314 => common::python_3145_command(),
                PythonVersion::Py315 => common::python_315_command(),
            };
            let native = python.args(["-c", oracle, source]).output().unwrap();
            assert!(
                !native.status.success(),
                "{version}: {source:?}: {native:?}"
            );
            let stdout = String::from_utf8(native.stdout).unwrap();
            let mut lines = stdout.lines();
            assert_eq!(
                lines.next(),
                Some(expected_message),
                "{version}: {source:?}"
            );
            let positions: Vec<usize> = lines
                .next()
                .unwrap()
                .split_whitespace()
                .map(|part| part.parse().unwrap())
                .collect();
            assert_eq!(positions.len(), 4);
            let offset = |line: usize, column: usize| {
                source
                    .split_inclusive('\n')
                    .take(line - 1)
                    .map(str::len)
                    .sum::<usize>()
                    + column
                    - 1
            };
            let native_start = offset(positions[0], positions[1]);
            let native_end = offset(positions[2], positions[3]);
            assert_eq!(&source[native_start..native_end], expected_span);

            let error = compile_with_options(source, "synthetic.py", &options).unwrap_err();
            let CompileError::InvalidSyntax { message, range } = error else {
                panic!("unexpected {version} error for {source:?}: {error}");
            };
            assert_eq!(message, expected_message, "{version}: {source:?}");
            assert_eq!(
                range.start().to_usize(),
                native_start,
                "{version}: {source:?}"
            );
            assert_eq!(range.end().to_usize(), native_end, "{version}: {source:?}");
        }
    }
}
