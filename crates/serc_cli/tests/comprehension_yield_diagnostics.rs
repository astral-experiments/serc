mod common;

use serc::{CompileOptions, PythonVersion, compile_bytes_with_options, compile_with_options};

fn byte_offset(source: &str, line: usize, column: usize) -> usize {
    let prefix: usize = source
        .split_inclusive('\n')
        .take(line - 1)
        .map(str::len)
        .sum();
    // These errors are raised by CPython's symbol table, whose columns are UTF-8 byte offsets.
    prefix + column - 1
}

#[test]
fn comprehension_yield_diagnostics_match_python() {
    let sources = [
        "def f():\n return [(yield x) for x in xs]\n",
        "def f():\n yield 0\n return [(yield x) for x in xs]\n",
        "def f():\n value = [(yield x) for x in xs]\n yield 0\n",
        "def f():\n return {(yield x) for x in xs}\n",
        "def f():\n return {(yield x): x for x in xs}\n",
        "def f():\n return {x: (yield x) for x in xs}\n",
        "def f():\n return {(yield x): (yield y) for x in xs}\n",
        "def f():\n return {(yield): (yield) for x in xs}\n",
        "def f():\n return {(yield 42): (yield from 'abc') for x in xs}\n",
        "def f():\n return {(yield from x): (yield from y) for x in xs}\n",
        "def f():\n return {(yield x): (yield from y) for x in xs}\n",
        "def f():\n return {(yield from x): (yield y) for x in xs}\n",
        "def f():\n return {(yield x): (await y) for x in xs}\n",
        "def f():\n return {(yield x): (lambda: (yield y)) for x in xs}\n",
        "def f():\n return {(lambda: (yield x)): (yield y) for x in xs}\n",
        "async def f():\n value = {(yield x): (yield from y) for x in xs}\n",
        "class C:\n values = {(yield x): (yield y) for x in xs}\n",
        "def f():\n return {(yield x): (yield y) for x in xs if (yield z)}\n",
        "def f():\n return {(yield x): (yield y) for x in (yield xs)}\n",
        "def f():\n return ((yield x) for x in xs)\n",
        "def f():\n return [x for x in xs if (yield x)]\n",
        "def f():\n return [y for x in xs for y in (yield x)]\n",
        "def f():\n return [(yield from x) for x in xs]\n",
        "def f():\n return [[(yield y) for y in ys] for x in xs]\n",
        "def f():\n return [[y for y in (yield ys)] for x in xs]\n",
        "def f():\n return [f'{(yield α)}' for x in xs]\n",
        "class C:\r\n values = [(yield α) for x in xs]\r\n",
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
fn yield_in_first_iterable_or_nested_lambda_remains_valid() {
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
            "def f():\n return [x for x in (yield xs)]\n",
            "def f():\n return (x for x in (yield xs))\n",
            "def f():\n return [(lambda: (yield x)) for x in xs]\n",
            "def f():\n return [lambda: (yield x) for x in xs]\n",
            "def f():\n return {(lambda: (yield x)): (lambda: (yield x)) for x in xs}\n",
            "def f():\n return {x: x + 1 for x in (yield xs)}\n",
        ] {
            assert!(
                compile_with_options(source, "case.py", &options).is_ok(),
                "{version}: {source:?}"
            );
            assert!(
                compile_bytes_with_options(source.as_bytes(), "case.py", &options).is_ok(),
                "{version}: {source:?}"
            );
        }
    }
}
