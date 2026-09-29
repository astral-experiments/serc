mod common;

use serc::{CompileOptions, PythonVersion};

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
        "class C:\n    result = [(yield [(q := 1) for y in ys]) for x in xs]\n",
        "def f():\n    return [(yield {0: (yield a), (yield b): 0}) for x in xs]\n",
        "def f():\n    return [(yield {(yield a): 0, 1: (yield b)}) for x in xs]\n",
        "def f():\n    return [(yield (lambda a=(yield x), a=1: 0)) for x in xs]\n",
        "def f():\n    return [(yield ((lambda a,a: 0), (yield x))) for x in xs]\n",
        "def f():\n    return [(yield [y for y in (lambda: (w := 1))]) for x in xs]\n",
        "def f():\n    return [(yield [y for y in (lambda p=(w := 1): p)]) for x in xs]\n",
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
    let nested = (0..40).fold(
        "{(k := 1): (yield v) for k in ks}".to_owned(),
        |expression, _| format!("[(yield {expression}) for x in xs]"),
    );
    let deep_source = format!("def f():\n    return {nested}\n");
    for version in common::PYTHON_VERSIONS {
        let options = CompileOptions {
            python_version: version,
            ..CompileOptions::default()
        };
        let modern_debug_after_star =
            matches!(version, PythonVersion::Py314 | PythonVersion::Py315)
                .then_some("*first\n[(yield (__debug__ := 1)) for x in xs]\n");
        for source in sources
            .into_iter()
            .chain(modern_debug_after_star)
            .chain(std::iter::once(deep_source.as_str()))
        {
            let diagnostic = common::diagnostics::syntax_diagnostic_with_bytes(version, source);
            let expected = diagnostic.byte_range(source);
            diagnostic.assert_syntax_error(source, &options, expected);
        }
    }
}
