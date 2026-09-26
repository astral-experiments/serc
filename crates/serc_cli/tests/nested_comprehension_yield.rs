mod common;

use serc::{CompileError, CompileOptions, PythonVersion, compile_bytes_with_options, compile_with_options};

fn byte_offset(source: &str, line: usize, column: usize) -> usize {
    let prefix: usize = source.split_inclusive('\n').take(line - 1).map(str::len).sum();
    prefix + column - 1
}

#[test]
fn nested_comprehension_yields_match_python() {
    let sources = [
        "def f():\n    return [(yield (yield x)) for x in xs]\n",
        "def f():\n    return [(yield from (yield x)) for x in xs]\n",
        "def f():\n    return [(yield (yield from x)) for x in xs]\n",
        "def f():\n    return [(yield from (yield from x)) for x in xs]\n",
        "def f():\n    return [(yield ((yield a), (yield b))) for x in xs]\n",
        "def f():\n    return [(yield (yield (yield z))) for x in xs]\n",
        "def f():\n    return {(yield (yield x)) for x in xs}\n",
        "def f():\n    return {x: (yield (yield x)) for x in xs}\n",
        "def f():\n    return ((yield (yield x)) for x in xs)\n",
        "def f():\n    return [(yield (lambda: (yield x))) for x in xs]\n",
        "def f():\n    return [(yield [y for y in (yield x)]) for x in xs]\n",
        "def f():\n    return [(yield {(yield y) for y in ys}) for x in xs]\n",
        "def f():\n    return [(yield {(yield k):(yield v) for k in ks}) for x in xs]\n",
        "def f():\n    return [(yield {(k := 1): (yield v) for k in ks}) for x in xs]\n",
        "def f():\n    return [(yield {(yield k): (k := 1) for k in ks}) for x in xs]\n",
        "class C:\n    result = [(yield {(x := 1): 0 for x in xs}) for z in zs]\n",
        "class C:\n    result = [(yield [[(b := 1) for c in cs] for b in bs]) for a in aa]\n",
        "def f():\n    return [(yield {0: (yield a), (yield b): 0}) for x in xs]\n",
        "def f():\n    return [(yield {(yield a): 0, 1: (yield b)}) for x in xs]\n",
        "def f():\n    return [(yield (lambda a=(yield x), a=1: 0)) for x in xs]\n",
        "def f():\n    return [(yield ((lambda a,a: 0), (yield x))) for x in xs]\n",
        "def f():\n    return [x for x in xs if (yield (yield x))]\n",
        "def f():\n    return [y for x in xs for y in (yield (yield x))]\n",
        "def f():\n    return [(yield ((await x), (yield y))) for x in xs]\n",
        "def f():\n    return [(yield (lambda: (await x))) for x in xs]\n",
        "def f():\n    return [(yield (lambda x=(await a): x)) for x in xs]\n",
        "def f():\n    return [(yield ((lambda: (await x)), (yield y))) for x in xs]\n",
        "def f():\n    return ((yield (lambda: (await x))) for x in xs)\n",
        "def f():\n    return ((yield (lambda x=(await a): x)) for x in xs)\n",
        "def f():\n    return [(yield *x) for x in xs]\n",
        "def f():\n    return [(yield *(yield x)) for x in xs]\n",
        "def f():\n    return [(yield ((g(a=1,a=2)), (yield x))) for x in xs]\n",
        "def f():\n    return [(yield ((__debug__ := 1), (yield x))) for x in xs]\n",
        "def f():\n    return [(yield ((yield x), (__debug__ := 1))) for x in xs]\n",
        "def f():\n    return [(yield (__debug__ := (yield x))) for x in xs]\n",
        "def f():\n    return [(yield (__debug__ := 1)) for x in xs]\n",
        "def f():\n    return [(yield (yield μ)) for x in xs]\n",
        "async def f():\n    value = [(yield from (yield x)) for x in xs]\n",
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
    for version in [PythonVersion::Py312, PythonVersion::Py313, PythonVersion::Py314, PythonVersion::Py315] {
        let options = CompileOptions { python_version: version, ..CompileOptions::default() };
        let modern_debug_after_star = matches!(version, PythonVersion::Py314 | PythonVersion::Py315)
            .then_some("*first\n[(yield (__debug__ := 1)) for x in xs]\n");
        for source in sources.into_iter().chain(modern_debug_after_star) {
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
            let positions: Vec<usize> = lines.next().unwrap().split_whitespace().map(|value| value.parse().unwrap()).collect();
            let expected = (byte_offset(source, positions[0], positions[1]), byte_offset(source, positions[2], positions[3]));
            for result in [compile_with_options(source, "case.py", &options), compile_bytes_with_options(source.as_bytes(), "case.py", &options)] {
                let error = result.unwrap_err();
                let CompileError::InvalidSyntax { message: actual_message, .. } = &error else {
                    panic!("{version}: {source:?}: expected a syntax restriction, got {error}");
                };
                assert_eq!(actual_message, message, "{version}: {source:?}");
                let range = error.range().unwrap();
                assert_eq!((range.start().to_usize(), range.end().to_usize()), expected, "{version}: {source:?}: {error}");
            }
        }
    }
}
