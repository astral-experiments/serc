use std::io::Write;
use std::process::{Command, Stdio};

use serc::{CompileOptions, PythonVersion, compile_with_options};

#[test]
fn private_generator_freevars_match_cpython() {
    let cases = [
        r"class C:
    def f(__x):
        return ((__x, _C__x) for i in [1])
g = C.f(42)
assert g.gi_code.co_freevars == ('_C__x',)
assert list(g) == [(42, 42)]
",
        r"def outer(_C__x):
    class C[T]:
        g = ((__x, _C__x) for i in [1])
    return C.g
g = outer(42)
assert g.gi_code.co_freevars == ('_C__x',)
assert list(g) == [(42, 42)]
",
        r"class C:
    def f(_Cb, __x):
        return ((_C__x, __x, _Cb) for i in [1])
g = C.f(5, 42)
assert g.gi_code.co_freevars == ('_C__x', '_Cb')
assert list(g) == [(42, 42, 5)]
",
        r"def f(_C__x):
    class C:
        g = ((__x, _C__x) for i in [1])
    return C.g
g = f(42)
assert g.gi_code.co_freevars == ('_C__x',)
assert list(g) == [(42, 42)]
",
        r"class C:
    def f(__x):
        return ((lambda: (__x, _C__x))() for i in [1])
g = C.f(42)
assert g.gi_code.co_freevars == ('_C__x',)
assert list(g) == [(42, 42)]
",
        r"class __C:
    def f(__z, __a, alpha):
        return ((__z, _C__z, __a, _C__a, alpha) for i in [1])
g = __C.f(3, 1, 2)
assert g.gi_code.co_freevars == ('_C__a', '_C__z', 'alpha')
assert list(g) == [(3, 3, 1, 1, 2)]
",
        r"class __:
    def f(__x, _C__x):
        return ((__x, _C__x) for i in [1])
g = __.f(1, 2)
assert g.gi_code.co_freevars == ('_C__x', '__x')
assert list(g) == [(1, 2)]
",
    ];
    let oracle = "import importlib.util, marshal, sys; assert sys.implementation.name == 'cpython'; assert sys.version.split()[0] == sys.argv[1]; assert importlib.util.MAGIC_NUMBER.hex() == sys.argv[2]; code = compile(sys.stdin.read(), 'private_generator.py', 'exec', dont_inherit=True, optimize=0); sys.stdout.buffer.write(marshal.dumps(code))";
    let run = "import marshal, sys; exec(marshal.loads(sys.stdin.buffer.read()))";
    for (version, variable, fallback) in [
        (PythonVersion::Py312, "SERC_TEST_PYTHON_312", "python3.12"),
        (PythonVersion::Py313, "SERC_TEST_PYTHON_313", "python3.13"),
        (PythonVersion::Py314, "SERC_TEST_PYTHON", "python3.14"),
        (PythonVersion::Py315, "SERC_TEST_PYTHON_315", "python3.15"),
    ] {
        let executable = std::env::var_os(variable).unwrap_or_else(|| fallback.into());
        let target = version.target();
        let options = CompileOptions {
            python_version: version,
            ..CompileOptions::default()
        };
        for (index, source) in cases.iter().enumerate() {
            let mut child = Command::new(&executable)
                .args(["-I", "-S", "-c", oracle])
                .args([target.tag.trim_start_matches('v'), &target.magic_hex()])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .expect("start the pinned CPython oracle");
            child
                .stdin
                .take()
                .unwrap()
                .write_all(source.as_bytes())
                .unwrap();
            let expected = child.wait_with_output().unwrap();
            assert!(
                expected.status.success(),
                "{}",
                String::from_utf8_lossy(&expected.stderr)
            );
            let actual = compile_with_options(source, "private_generator.py", &options)
                .unwrap()
                .marshal();
            let first_difference = actual
                .iter()
                .zip(&expected.stdout)
                .position(|(a, b)| a != b);
            assert!(
                actual == expected.stdout,
                "{version:?}, case {index}, first difference {first_difference:?}, lengths {} and {}",
                actual.len(),
                expected.stdout.len()
            );
            let mut child = Command::new(&executable)
                .args(["-I", "-S", "-c", run])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .expect("start the pinned CPython runtime");
            child.stdin.take().unwrap().write_all(&actual).unwrap();
            let result = child.wait_with_output().unwrap();
            assert!(
                result.status.success(),
                "{version:?}, case {index}: {}",
                String::from_utf8_lossy(&result.stderr)
            );
        }
    }
}
