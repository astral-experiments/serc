mod common;

use serc::{CompileOptions, compile_bytes_with_options, compile_with_options};

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
        "async def f():\n return {(yield x): (yield from y) for x in xs}\n",
        "async def f():\n return [(yield x) for x in xs]\n",
        "async def f():\n return {(yield x) for x in xs}\n",
        "async def f():\n return ((yield x) for x in xs)\n",
        "async def f():\n yield 1\n return [(yield x) for x in xs]\n",
        "async def f():\n return [(yield x) for x in xs]\n yield 1\n",
        "async def f():\n yield 1\n return 2\n [(yield x) for x in xs]\n",
        "async def f():\n return 2\n [(yield x) for x in xs]\n",
        "async def f():\n yield 1\n return 2\ndef g():\n return [(yield x) for x in xs]\n",
        "def g():\n return [(yield x) for x in xs]\nasync def f():\n yield 1\n return 2\n",
        "async def f():\n yield 1\n return 2\n[(yield x) for x in xs]\n",
        "async def f():\n yield 1\n return 2\n(yield x)\n[(yield x) for x in xs]\n",
        "async def f():\n yield 1\n return 2\n(yield from x)\n[(yield x) for x in xs]\n",
        "async def f():\n yield 1\n return 2\ndef f():\n return [(yield x) for x in xs]\n",
        "async def f():\n return [(yield x) for x in xs for y in ys]\n",
        "async def f(x, /, y=1, *args, z=None, **kwargs):\n return {(yield x): (yield from y) for x in xs}\n",
        "async def f(x):\n value = 1\n yield x\n return 2\n later = [(yield x) for x in xs]\n",
        "async def f():\n yield 1\n return 2\nfirst = second = [(yield x) for x in xs]\n",
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
        "*first\n[(yield y) for y in ys]\n",
        "*first\n[(yield from y) for y in ys]\n",
        "*first\n[(yield (yield y)) for y in ys]\n",
        "*first\n[(yield (__debug__ := 1)) for x in xs]\n",
        "*first\n[(yield from (__debug__ := 1)) for x in xs]\n",
        "*first\n{(yield y) for y in ys}\n",
        "*first\n{(yield y): y for y in ys}\n",
        "*first\n((yield y) for y in ys)\n",
        "*first\n[y for y in ys if (yield y)]\n",
        "*first\n[y for x in xs for y in (yield x)]\n",
        // These yields belong to the enclosing scope or a nested lambda, not the comprehension.
        "*first\n(yield y)\n",
        "*first\n[x for x in (yield xs)]\n",
        "*first\n[(lambda: (yield x)) for x in xs]\n",
        "def f():\n *first\n [x for x in (yield xs)]\n",
    ];
    for version in common::PYTHON_VERSIONS {
        let options = CompileOptions {
            python_version: version,
            ..CompileOptions::default()
        };
        for source in sources {
            let diagnostic = common::diagnostics::syntax_diagnostic_with_bytes(version, source);
            let normalized = source.replace("\r\n", "\n");
            let expected = diagnostic.byte_range(&normalized);
            diagnostic.assert_error(source, &options, expected);
        }
    }
}

#[test]
fn yield_in_first_iterable_or_nested_lambda_remains_valid() {
    for version in common::PYTHON_VERSIONS {
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
