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
    check_fixture_on(source, exercise, &common::PYTHON_VERSIONS);
}

#[cfg(feature = "decompile")]
fn check_fixture_on(source: &str, exercise: &str, versions: &[serc::PythonVersion]) {
    let temp = tempfile::tempdir().unwrap();
    let original = temp.path().join("original.py");
    let recovered = temp.path().join("recovered.py");
    let cache = temp.path().join("file.pyc");
    fs_err::write(&original, source).unwrap();
    for &version in versions {
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

#[cfg(feature = "decompile")]
#[test]
fn suspension_and_iteration() {
    check_fixture(
        "def produce(xs):\n    yield 1\n    yield from xs\n",
        "result = list(produce([2,3]))",
    );
    check_fixture(
        "async def fetch(task):\n    return await task()\n",
        "import asyncio\nasync def task(): return 42\nresult = asyncio.run(fetch(task))",
    );
    check_fixture(
        "def comp(xs):\n    return [x+1 for x in xs if x]\n",
        "result = comp([0,1,2])",
    );
    check_fixture(
        "def total(xs):\n    return sum(x+1 for x in xs if x)\n",
        "result = total([0,1,2])",
    );
    check_fixture(
        "async def consume(xs, log):\n    async for x in xs:\n        log(x)\n",
        "import asyncio\nasync def xs():\n    yield 1\n    yield 2\nresult=[]\nasyncio.run(consume(xs(), result.append))",
    );
    check_fixture(
        "async def manage(cm, log):\n    async with cm as value:\n        log(value)\n",
        "import asyncio, contextlib\n@contextlib.asynccontextmanager\nasync def manager(): yield 42\nresult=[]\nasyncio.run(manage(manager(),result.append))",
    );
}

#[cfg(feature = "decompile")]
#[test]
fn captures_guards_and_format_specs() {
    check_fixture(
        "def f(x):\n    match x:\n        case [a, b] if a: return b\n        case 4: return x\n        case _: return None\n",
        "result = [f(x) for x in ([1,2], [0,2], 4, {}, None)]",
    );
    check_fixture(
        "def f(x):\n    match x:\n        case {'hi': a}: return a\n        case _: return False\n",
        "result = [f(x) for x in ({'hi': 42}, {}, [])]",
    );
    check_fixture(
        "def f(x):\n    match x:\n        case A(a, v=b): return a+b\n        case _: return x\n",
        "from collections import namedtuple\nA = namedtuple('A', 'u v')\nresult = f(A(1,2)), f(4)",
    );
    check_fixture(
        "def f(x):\n    return f'{{hi}} {x!r:>8}! {x}'\n",
        "result = f(42)",
    );
}

#[cfg(feature = "decompile")]
#[test]
fn template_interpolations_retain_expression_text() {
    check_fixture_on(
        "def f(n):\n    return t'{{hi}} {n!r:>8} {n}'\n",
        "t = f(42)\nresult = t.strings, [(i.value, i.expression, i.conversion, i.format_spec) for i in t.interpolations]",
        &[serc::PythonVersion::Py314, serc::PythonVersion::Py315],
    );
}

#[cfg(feature = "decompile")]
#[test]
fn corpus_operand_layouts() {
    check_fixture(
        "import os\nimport sys as alias\nx: int = 3\ny: str\nclass C:\n    n: int = 42\n    def read(self, k: int) -> int:\n        return self.n + k\n",
        "result = (x, C().read(2), C.__annotations__, (__annotate__(1) if \"__annotate__\" in globals() else __annotations__), alias is __import__('sys'))",
    );
    check_fixture(
        "def f(obj, call, xs):\n    obj.attr = call(1, k=2)\n    xs[1:3] = [4, 5, 6]\n    a, b = xs[:2]\n    return a + b, obj.attr\n",
        "from types import SimpleNamespace\nresult = f(SimpleNamespace(), lambda a, k: a + k, [0, 1, 2])",
    );
}

#[cfg(feature = "decompile")]
#[test]
fn extended_calls_and_assignment_expressions() {
    check_fixture(
        "def f(call, args, kw):\n    return call(1, *args, k=2, **kw)\n",
        "result = f(lambda *a, **kw: (a, kw), (3, 4), {'z': 5})",
    );
    check_fixture(
        "from math import *\ndef f(n):\n    assert n, 'missing'\n    n += 1\n    return (lambda x: x + 2)(v := n), v\n",
        "result = f(2), sqrt(9)",
    );
    check_fixture(
        "def f(x):\n    return x in {1, 3, 5}, 0x123456789abcdef0001234\n",
        "result = f(3), f(7)",
    );
}
