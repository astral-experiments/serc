mod common;

use serc::{
    CompileError, CompileOptions, PythonVersion, compile_bytes_with_options, compile_with_options,
};

fn byte_offset(source: &str, line: usize, column: usize) -> usize {
    source
        .split_inclusive('\n')
        .take(line - 1)
        .map(str::len)
        .sum::<usize>()
        + column
        - 1
}

#[test]
fn await_outside_async_function_matches_python() {
    let sources = [
        "def f():\n    await g()\n",
        "def f():\n    yield 1\n    await g()\n",
        "def f():\n    return lambda: (await g())\n",
        "def f():\n    return lambda x=(await g()): x\n",
        "f = lambda x=(await g()): x\n",
        "class C:\n    f = lambda x=(await g()): x\n",
        "async def f():\n    def inner():\n        await g()\n",
        "def f():\n    return [lambda: (await g()) for x in xs]\n",
        "def f():\n    return [x for x in await g()]\n",
        "await g()\n",
        "class C:\n    await g()\n",
        "def f():\n    class C:\n        await g()\n",
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
            let expected = (
                byte_offset(source, positions[0], positions[1]),
                byte_offset(source, positions[2], positions[3]),
            );
            for result in [
                compile_with_options(source, "case.py", &options),
                compile_bytes_with_options(source.as_bytes(), "case.py", &options),
            ] {
                let error = result.unwrap_err();
                let CompileError::InvalidSyntax {
                    message: actual, ..
                } = &error
                else {
                    panic!("{version}: {source:?}: expected syntax error, got {error}");
                };
                assert_eq!(actual, message, "{version}: {source:?}");
                let range = error.range().unwrap();
                assert_eq!(
                    (range.start().to_usize(), range.end().to_usize()),
                    expected,
                    "{version}: {source:?}"
                );
            }
        }
        for source in [
            "async def f():\n    await g()\n",
            "async def f():\n    return [await g(x) for x in xs]\n",
            "async def f():\n    return lambda x=(await g()): x\n",
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
