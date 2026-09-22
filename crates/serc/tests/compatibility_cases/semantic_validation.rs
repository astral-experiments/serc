use serc::{CompileError, compile};

use crate::common::{assert_matches_cpython_marshal, python_3145_command};

#[test]
fn rejects_semantic_errors_before_discarding_code() {
    for source in [
        "if False:\n    break\n",
        "type X = (y := x for x in xs)\n",
        "type X = (x for x in await xs)\n",
        "type X = lambda: await x\n",
        "if False:\n    return 1\n",
        "def f():\n    if False:\n        await x\n",
        "while False:\n    def f():\n        break\n",
        "def f():\n    class C:\n        return 1\n",
        "if False:\n    from __future__ import annotations\n",
        "if False:\n    def f(x, x): pass\n",
        "if False:\n    match x:\n        case [a] | [b]: pass\n",
        "def f():\n    return\n    async for x in xs: pass\n",
        "def f():\n    return\n    values = [x for x in xs if await x]\n",
        "from __future__ import annotations\ndef f(x: (lambda y=(yield 1): y)): pass\n",
    ] {
        let expected = python_3145_command()
            .args([
                "-c",
                "import sys; compile(sys.argv[1], 'semantic.py', 'exec')",
                source,
            ])
            .output()
            .unwrap();
        assert!(!expected.status.success(), "CPython accepted {source}");
        assert!(String::from_utf8_lossy(&expected.stderr).contains("SyntaxError:"));
        let error = compile(source, "semantic.py").unwrap_err();
        assert!(
            matches!(
                error,
                CompileError::InvalidSyntax { .. } | CompileError::InvalidPattern { .. }
            ),
            "expected a source diagnostic for {source}, got {error:?}"
        );
        assert!(error.range().is_some_and(|range| !source[range].is_empty()));
    }
}

#[test]
fn semantic_validation_respects_scope_boundaries() {
    for source in [
        "from .__future__ import annotations\nx = 1\n",
        "def f():\n    yield 1\n    def g(x=(yield 2)): pass\n",
        "async def f():\n    def g(x=await value): pass\n",
        "def f():\n    return (await x for x in xs)\n",
        "def f():\n    return (x async for x in xs)\n",
        "async def f():\n    return [[x async for x in xs] for xs in groups]\n",
        "from __future__ import annotations\ndef outer():\n    def inner(x: (lambda: (yield 1))): pass\n",
        "type X = lambda: (yield 1)\n",
        "type X = lambda: (x := 1)\n",
        "type X = lambda: (await x for x in xs)\n",
        "type X = (await x for x in xs)\n",
        "type X = (x async for x in xs)\n",
        "from __future__ import annotations\ndef f(x: (await y for y in ys)): pass\n",
        "def f[T: (await x for x in xs)](): pass\n",
        "def f[T: (lambda: (yield 1))](): pass\n",
        "while outer:\n    while inner: pass\n    else: break\n",
    ] {
        assert_matches_cpython_marshal(source, "semantic.py");
    }
}
