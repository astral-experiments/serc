use serc::{CompileOptions, PythonVersion, compile_bytes_with_options, compile_with_options};

const SOURCE: &str =
    "def f(a, b):\n    try:\n        g()\n    finally:\n        assert a is b and not a.value\n";
const FILENAME: &str = "exceptional_finally_assert_success.py";

#[test]
fn matches_cpython_312_exceptional_finally_assert_success() {
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
    let options = CompileOptions {
        python_version: version,
        ..CompileOptions::default()
    };
    for source in [
        SOURCE,
        "def f(a, b, c):\n    try:\n        g()\n    finally:\n        assert a is b and a.value\n",
        "def f(a, b, c):\n    try:\n        g()\n    finally:\n        assert a\n",
        "def f(a, b, c):\n    try:\n        g()\n    finally:\n        assert a and not True\n",
        "def f(a, b, c):\n    try:\n        g()\n    finally:\n        assert a is b and not a.value\n        g()\n",
        "def f(a, b, c):\n    for _ in range(2):\n        try:\n            g()\n        finally:\n            assert a is b and not a.value\n",
        "def f(a, b, c):\n    try:\n        yield a\n    finally:\n        assert a is b and not a.value\n",
    ] {
        let expected = crate::common::cpython::run_with_input(
            crate::common::cpython::command(version)
                .args(["-I", "-S", "-X", "utf8", "-c", oracle])
                .args([&target.magic_hex(), FILENAME]),
            source.as_bytes(),
        );
        assert!(
            expected.status.success(),
            "CPython 3.12 oracle failed: {}",
            String::from_utf8_lossy(&expected.stderr)
        );
        assert_eq!(
            compile_with_options(source, FILENAME, &options)
                .unwrap()
                .marshal(),
            expected.stdout,
            "text source: {source}"
        );
        assert_eq!(
            compile_bytes_with_options(source.as_bytes(), FILENAME, &options)
                .unwrap()
                .marshal(),
            expected.stdout,
            "byte source: {source}"
        );
    }
}
