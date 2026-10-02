use serc::{CompileOptions, PythonVersion, compile_bytes_with_options, compile_with_options};

#[path = "common/cpython.rs"]
mod cpython;

fn assert_matches_cpython(source: &str) {
    let version = PythonVersion::Py312;
    let target = version.target();
    let oracle = r"
import importlib.util, marshal, sys
assert sys.implementation.name == 'cpython'
assert sys.version_info == (3, 12, 14, 'final', 0)
assert importlib.util.MAGIC_NUMBER.hex() == sys.argv[1]
code = compile(sys.stdin.buffer.read(), sys.argv[2], 'exec', dont_inherit=True, optimize=0)
sys.stdout.buffer.write(marshal.dumps(code))
";
    let filename = "folded_assertion_exits.py";
    let expected = cpython::run_with_input(
        cpython::command(version)
            .args(["-B", "-I", "-S", "-X", "utf8", "-c", oracle])
            .args([&target.magic_hex(), filename]),
        source.as_bytes(),
    );
    assert!(
        expected.status.success(),
        "CPython oracle failed: {}",
        String::from_utf8_lossy(&expected.stderr)
    );
    let options = CompileOptions {
        python_version: version,
        ..CompileOptions::default()
    };
    assert_eq!(
        compile_with_options(source, filename, &options)
            .unwrap()
            .marshal(),
        expected.stdout,
        "text source: {source}"
    );
    assert_eq!(
        compile_bytes_with_options(source.as_bytes(), filename, &options)
            .unwrap()
            .marshal(),
        expected.stdout,
        "byte source: {source}"
    );
}

fn check_contexts(contexts: &[&str]) {
    for condition in [
        "True",
        "1",
        "-1",
        "'ready'",
        "b'x'",
        "...",
        "__debug__",
        "1 + 2",
        "not False",
        "(1, 2)",
    ] {
        for message in ["", ", explain(value)"] {
            let assertion = format!("assert {condition}{message}");
            for context in contexts {
                assert_matches_cpython(&context.replace("ASSERT", &assertion));
            }
        }
    }
}

#[test]
fn folded_assertion_success_copies_returns_only_after_constant_folding() {
    check_contexts(&[
        "def f(value):\n    ASSERT\n",
        "def f(value):\n    if value:\n        ASSERT\n    return value\n",
        "def f(value):\n    if value:\n        ASSERT\n    return None\n",
        "def f(value):\n    assert value is None or True\n    ASSERT\n",
    ]);
}

#[test]
fn folded_assertion_keeps_cleanup_predecessors_shared() {
    check_contexts(&[
        "def f(value):\n    with value:\n        work()\n    ASSERT\n",
        "def f(value):\n    with value:\n        ASSERT\n",
        "def f(value):\n    try:\n        work()\n    except Exception:\n        pass\n    ASSERT\n",
    ]);
}

#[test]
fn folded_literal_assertion_preserves_copied_finalizer_exits() {
    for condition in ["True", "1", "'ready'", "1 + 2"] {
        for message in ["", ", explain(value)"] {
            let source = format!(
                "def f(value):\n    try:\n        work()\n    finally:\n        assert {condition}{message}\n"
            );
            assert_matches_cpython(&source);
        }
    }
}

#[test]
fn folded_assertion_preserves_suspending_cleanup_exits() {
    check_contexts(&[
        "def f(value):\n    with value:\n        yield value\n    ASSERT\n",
        "def f(value):\n    try:\n        yield value\n    finally:\n        finish()\n    ASSERT\n    return None\n",
        "async def f(value):\n    await value\n    ASSERT\n",
        "async def f(value):\n    async with value:\n        work()\n    ASSERT\n",
        "async def f(value):\n    async with value:\n        yield value\n    ASSERT\n",
    ]);
}
