mod common;

use serc::{CompileError, CompileOptions, PythonVersion, compile_with_options};

#[test]
fn future_import_diagnostics_follow_native_order() {
    const LATE: &str = "from __future__ imports must occur at the beginning of the file";
    const UNKNOWN: &str = "future feature imaginary is not defined";
    const BRACES: &str = "not a chance";
    const REPEAT: &str = "keyword argument repeated: x";
    let cases = [
        (
            "x = 1\nfrom __future__ import imaginary\n",
            (LATE, "from __future__ import imaginary"),
            (LATE, "from __future__ import imaginary"),
        ),
        (
            "from __future__ import imaginary\nx = 1\nfrom __future__ import another_unknown\n",
            (UNKNOWN, "imaginary"),
            (UNKNOWN, "imaginary"),
        ),
        (
            "from __future__ import braces\n",
            (BRACES, "braces"),
            (BRACES, "braces"),
        ),
        (
            "x = 1\nfrom __future__ import braces\n",
            (LATE, "from __future__ import braces"),
            (LATE, "from __future__ import braces"),
        ),
        (
            "from __future__ import annotations, imaginary\n",
            (UNKNOWN, "imaginary"),
            (UNKNOWN, "imaginary"),
        ),
        (
            "x = 1\nfrom __future__ import annotations, imaginary\n",
            (LATE, "from __future__ import annotations, imaginary"),
            (LATE, "from __future__ import annotations, imaginary"),
        ),
        (
            "from __future__ import imaginary, braces\n",
            (UNKNOWN, "imaginary"),
            (UNKNOWN, "imaginary"),
        ),
        (
            "x = 1\nfrom __future__ import imaginary, braces\n",
            (LATE, "from __future__ import imaginary, braces"),
            (LATE, "from __future__ import imaginary, braces"),
        ),
        (
            "from __future__ import braces, imaginary\n",
            (BRACES, "braces"),
            (BRACES, "braces"),
        ),
        (
            "x = 1\nfrom __future__ import braces, imaginary\n",
            (LATE, "from __future__ import braces, imaginary"),
            (LATE, "from __future__ import braces, imaginary"),
        ),
        (
            "from __future__ import annotations\nx = 1\nfrom __future__ import imaginary\n",
            (LATE, "from __future__ import imaginary"),
            (LATE, "from __future__ import imaginary"),
        ),
        (
            "f(x=1,x=2)\nfrom __future__ import imaginary\n",
            (REPEAT, "x=2"),
            (LATE, "from __future__ import imaginary"),
        ),
        (
            "if False:\n    from __future__ import imaginary\n",
            (LATE, "from __future__ import imaginary"),
            (LATE, "from __future__ import imaginary"),
        ),
        (
            "def f():\n    from __future__ import imaginary\n",
            (LATE, "from __future__ import imaginary"),
            (LATE, "from __future__ import imaginary"),
        ),
        (
            "from __future__ import imaginary\nfrom __future__ import braces\n",
            (UNKNOWN, "imaginary"),
            (UNKNOWN, "imaginary"),
        ),
        (
            "from __future__ import braces\nfrom __future__ import imaginary\n",
            (BRACES, "braces"),
            (BRACES, "braces"),
        ),
        (
            "x = 1\nfrom __future__ import annotations, braces, imaginary\n",
            (
                LATE,
                "from __future__ import annotations, braces, imaginary",
            ),
            (
                LATE,
                "from __future__ import annotations, braces, imaginary",
            ),
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
            assert_eq!(
                &source[native_start..native_end],
                expected_span,
                "{version}: {source:?}"
            );
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
