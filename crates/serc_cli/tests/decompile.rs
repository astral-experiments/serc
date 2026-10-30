#[cfg(feature = "decompile")]
mod common;

use std::process::Command;

#[cfg(not(feature = "decompile"))]
#[test]
fn command_requires_feature() {
    let output = Command::new(env!("CARGO_BIN_EXE_serc"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(
        !String::from_utf8(output.stdout)
            .unwrap()
            .contains("decompile")
    );
}

#[cfg(feature = "decompile")]
#[test]
fn complete_modules_only() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("original.py");
    let cache = temp.path().join("module.pyc");
    for version in common::PYTHON_VERSIONS {
        for optimize in 0..=2 {
            for supported in [true, false] {
                let text = if supported {
                    "def add(x, y):\n    return x + y\nanswer = add(20, 22)\n".to_owned()
                } else {
                    format!(
                        "answer = 1\ndef unsupported():\n    return '{}'\n",
                        "x".repeat(200_000)
                    )
                };
                fs_err::write(&source, &text).unwrap();
                let built = common::python_command(version).args(["-X", "no_debug_ranges", "-c", "import py_compile, sys; py_compile.compile(sys.argv[1], sys.argv[2], optimize=int(sys.argv[3]), doraise=True)"]).arg(&source).arg(&cache).arg(optimize.to_string()).output().unwrap();
                assert!(built.status.success(), "{built:?}");
                fs_err::remove_file(&source).unwrap();
                let recovered = Command::new(env!("CARGO_BIN_EXE_serc"))
                    .arg("decompile")
                    .arg(&cache)
                    .output()
                    .unwrap();
                if supported {
                    assert!(recovered.status.success(), "{version}: {recovered:?}");
                    assert_eq!(String::from_utf8(recovered.stdout).unwrap(), text);
                } else {
                    assert!(!recovered.status.success());
                    assert!(recovered.stdout.is_empty());
                    let message = String::from_utf8(recovered.stderr).unwrap();
                    assert!(
                        message.contains("unsupported")
                            && message.contains("byte")
                            && message.contains(&version.to_string()),
                        "{message}"
                    );
                }
            }
        }
    }
}

#[cfg(feature = "decompile")]
fn check_fixture(source: &str, exercise: &str) {
    let temp = tempfile::tempdir().unwrap();
    let original = temp.path().join("original.py");
    let recovered = temp.path().join("recovered.py");
    let cache = temp.path().join("file.pyc");
    fs_err::write(&original, source).unwrap();
    for version in common::PYTHON_VERSIONS {
        let built = common::python_command(version).args(["-c", "import py_compile, sys; py_compile.compile(sys.argv[1], sys.argv[2], doraise=True)"]).arg(&original).arg(&cache).output().unwrap();
        assert!(built.status.success(), "{built:?}");
        let output = Command::new(env!("CARGO_BIN_EXE_serc"))
            .arg("decompile")
            .arg(&cache)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{version}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        fs_err::write(&recovered, output.stdout).unwrap();
        let oracle = common::python_command(version)
            .args([
                "-c",
                r"
import sys, types
def load(path):
    return compile(open(path, encoding='utf-8').read(), '<fixture>', 'exec')
def fields(c):
    return (c.co_code, c.co_flags, c.co_names, c.co_varnames, c.co_cellvars, c.co_freevars,
            c.co_argcount, c.co_posonlyargcount, c.co_kwonlyargcount, c.co_exceptiontable,
            tuple(fields(k) if isinstance(k, types.CodeType) else k for k in c.co_consts))
def exercise(c):
    scope = {}
    exec(c, scope)
    exec(sys.argv[3], scope)
    return scope['result']
left, right = map(load, sys.argv[1:3])
assert fields(left) == fields(right)
assert exercise(left) == exercise(right)
",
            ])
            .arg(&original)
            .arg(&recovered)
            .arg(exercise)
            .output()
            .unwrap();
        assert!(
            oracle.status.success(),
            "{version}: {}",
            String::from_utf8_lossy(&oracle.stderr)
        );
    }
}

#[cfg(feature = "decompile")]
#[test]
fn branches_preserve_call_order() {
    check_fixture(
        "def loop(xs, emit):\n    for x in xs:\n        if x < 0:\n            continue\n        if x == 2:\n            break\n        emit(x)\n    else:\n        emit(0)\n    emit(9)\n    return 1\n",
        "log = []\nloop([-1, 1, 2], log.append)\nloop([],log.append)\nresult = log",
    );
    check_fixture(
        "def f(xs, emit):\n    for x in xs:\n        emit(x)\n    return xs and emit(7)\n",
        "log = []\nf([], log.append)\nf([1, 2], log.append)\nresult = log",
    );
    check_fixture(
        "def f(x, ready, work):\n    while ready(x):\n        work(x)\n    return x\n",
        "calls = []\nf(1, lambda x: len(calls) < 2, calls.append)\nresult = calls",
    );
}

#[cfg(feature = "decompile")]
#[test]
fn definitions_preserve_bindings() {
    check_fixture(
        "def outer(x=3, *, k=4):\n    def inner(y):\n        return x + y\n    return inner\n",
        "result = outer()(2)",
    );
    check_fixture(
        "class C:\n    def f(self, name='x'):\n        return name\n",
        "result = C().f()",
    );
    check_fixture(
        "from sys import version as v\nimport sys as s\n",
        "result = v == s.version",
    );
    check_fixture(
        "def deco(f):\n    return f\n@deco\ndef f(x, /, *args, y=3, **kwargs):\n    return x\n",
        "result = f(42,1,2)",
    );
    check_fixture(
        "def f[T](x: T) -> T:\n    return x\n",
        "result = f(42), repr(f.__annotations__)",
    );
}

#[cfg(feature = "decompile")]
#[test]
fn cleanup_on_normal_and_exceptional_paths() {
    check_fixture(
        "def f(work, emit):\n    try:\n        return work()\n    finally:\n        emit(1)\n",
        "log = []\nf(lambda: 42, log.append)\ntry: f(lambda: 1/0, log.append)\nexcept ZeroDivisionError: log.append(2)\nresult = log",
    );
    check_fixture(
        "def f(work, emit):\n    try:\n        work()\n    except ValueError:\n        emit(1)\n    emit(2)\n",
        "log = []\nf(lambda: None,log.append)\nf(lambda: int('x'),log.append)\nresult = log",
    );
    check_fixture(
        "def f(cm, work):\n    with cm as value:\n        work(value)\n    return 42\n",
        "import contextlib\nresult = f(contextlib.suppress(ZeroDivisionError),lambda _: 1/0)",
    );
    check_fixture(
        "def f(work, log):\n    try:\n        work()\n    except* ValueError:\n        log(1)\n    log(2)\n",
        "log=[]\ndef work(): raise ExceptionGroup('group',[ValueError()])\nf(work,log.append)\nresult=log",
    );
}
