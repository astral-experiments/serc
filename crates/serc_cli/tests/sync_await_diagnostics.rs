mod common;

use serc::{CompileOptions, compile_bytes_with_options, compile_with_options};

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
    for version in common::PYTHON_VERSIONS {
        let options = CompileOptions {
            python_version: version,
            ..CompileOptions::default()
        };
        for source in sources {
            let diagnostic = common::diagnostics::syntax_diagnostic(version, source);
            let expected = diagnostic.byte_range(source);
            diagnostic.assert_syntax_error(source, &options, expected);
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
