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

#[cfg(feature = "decompile")]
#[test]
fn dotted_import_and_future_annotation_spellings() {
    check_fixture(
        "from __future__ import annotations\nimport xml.etree.ElementTree as tree\ndef f(x: int) -> (int, str):\n    return x\n",
        "result = tree.Element('tag').tag, f(7), f.__annotations__",
    );
}

#[cfg(feature = "decompile")]
#[test]
fn discarded_statements_keep_tracing_instructions() {
    check_fixture("pass\npass\npass\n", "result = 1");
    check_fixture(
        "def f():\n    pass\n    pass\n    return 1\n",
        "result = f()",
    );
}

#[cfg(feature = "decompile")]
#[test]
fn nested_context_managers_unwind_in_reverse_order() {
    check_fixture(
        "def f(outer, middle, inner, work):\n    with outer as a, middle as b, inner:\n        work(a, b)\n    return 42\n",
        r"
log = []
class Manager:
    def __init__(self, name, suppress=False):
        self.name, self.suppress = name, suppress
    def __enter__(self):
        log.append(('enter', self.name))
        return self.name
    def __exit__(self, exc, value, tb):
        log.append(('exit', self.name, exc.__name__ if exc else None))
        return self.suppress
def fail(a, b):
    log.append((a, b))
    raise ValueError()
normal = f(Manager('a'), Manager('b'), Manager('c'), lambda a,b: log.append((a,b)))
suppressed = f(Manager('a'), Manager('b', True), Manager('c'), fail)
result = normal, suppressed, log",
    );
    check_fixture(
        "def f(a, b, work):\n    with a, b:\n        return work()\n",
        "from contextlib import nullcontext, suppress\nresult = f(nullcontext(), nullcontext(), lambda: 17), f(nullcontext(), suppress(ValueError), lambda: int('x'))",
    );
}

#[cfg(feature = "decompile")]
#[test]
fn assignment_targets_are_evaluated_once() {
    check_fixture(
        "def update(obj, key, value):\n    obj().attr += value()\n    obj()[key()] *= value()\n    obj()[key():key()] += [value()]\n    del obj().attr\n    del obj()[key()]\n",
        r"
log = []
class Box(list):
    attr = 10
box = Box([2,3,4])
def obj():
    log.append('obj')
    return box
def key():
    log.append('key')
    return 1
def value():
    log.append('value')
    return 3
update(obj,key,value)
result = list(box), box.attr, log",
    );
    check_fixture(
        "v = 1\ndef outer():\n    x = 2\n    def inner():\n        global v\n        nonlocal x\n        del x, v\n        t = 3\n        del t\n    inner()\n",
        "outer()\nresult = 'v' in globals()",
    );
}

#[cfg(feature = "decompile")]
#[test]
fn unpacking_targets_preserve_binding_order_and_scope() {
    check_fixture(
        "def assign(rows):\n    (first, last), (*span, tail) = rows\n    return first, last, span, tail\ndef loop(rows, emit):\n    for head, (a, *middle, z) in rows:\n        emit((head, a, middle, z))\n",
        "log = []\nloop([(0, [1, 2, 3, 4]), (5, [6, 7])], log.append)\nresult = assign(((1,2),(3,4,5))), log",
    );
    check_fixture(
        "def comp(rows):\n    a = 'outer'\n    values = {a: b for a, *b in rows if a}\n    return values, a\ndef gen(rows):\n    return (a + b for a, b in rows)\n",
        "result = comp([(0,1), (2,3,4), (5,)]), list(gen([(1,2), (4,5)]))",
    );
    check_fixture(
        "def manage(cm, log):\n    with cm as (a, *rest):\n        log((a, rest))\nasync def consume(rows, log):\n    async for a, (b, *c) in rows:\n        log((a, b, c))\nasync def manage_async(cm, log):\n    async with cm as (a, b):\n        log((a, b))\n",
        "import asyncio, contextlib\nlog=[]\nmanage(contextlib.nullcontext((1,2,3)), log.append)\nasync def rows(): yield (1, (2,3,4))\nasyncio.run(consume(rows(), log.append))\nasyncio.run(manage_async(contextlib.nullcontext((4,5)), log.append))\nresult=log",
    );
}

#[cfg(feature = "decompile")]
#[test]
fn comprehension_clauses_preserve_iteration_and_filter_order() {
    check_fixture(
        "def f(rows, project, select):\n    return [(a, b if b else 0) for a, values in rows if select(a) for b in project(values) if b is None or select(b)]\n",
        r"
log = []
def select(a):
    log.append(('select', a))
    return a
def project(values):
    log.append(('project', values))
    return values
result = f([(0, [9]), (1, [None, 0, 3]), (2, [])], project, select), log",
    );
    check_fixture(
        "def mapping(rows):\n    return {key: item for row in rows if row for key, items in row for item in items if not item}\ndef unique(groups):\n    return {item for group in groups for item in group if item is None}\n",
        "result = mapping([[], [('a', [1, 0]), ('b', [None])]]), unique([[1,None],[],[None]])",
    );
    check_fixture(
        "def nested(rows):\n    x = 'outer'\n    result = [[x for x in row] for row in rows]\n    return result, x\n",
        "result = nested([[1,2],[],[3]])",
    );
    check_fixture(
        "def gen(rows, visit):\n    return (visit(b) for a in rows if a for b in a if b)\n",
        r"
log = []
def rows():
    for row in ([], [0, 4, 5], [6]):
        log.append(('row', row))
        yield row
def visit(value):
    log.append(('visit', value))
    return value
g = gen(rows(), visit)
before = list(log)
first = next(g)
after_first = list(log)
result = before, first, after_first, list(g), log",
    );
}

#[cfg(feature = "decompile")]
#[test]
fn starred_collections_evaluate_elements_in_order() {
    check_fixture(
        "def build(f, spread, call):\n    a = [f(1), *spread(2), f(3)]\n    b = {f(4), *spread(5), f(6)}\n    c = (f(7), *spread(8), f(9))\n    d = call(f(10), *spread(11), f(12), token=f(13))\n    return a, b, c, d\n",
        r"
log = []
def f(n):
    log.append(n)
    return n
def spread(n):
    log.append(n)
    yield n
    log.append(-n)
    yield -n
result = build(f, spread, lambda *args, **kw: (args, kw)), log",
    );
    check_fixture(
        "def f(x, y):\n    return [*(x(), y()), 1], {*(1, 2), x()}, (3, *y(), (4,5)), [1, 2, 3], {1, 2, 3}, {*{1, 2, 3}, x()}\n",
        "result = f(lambda: 42, lambda: (0,))",
    );
}

#[cfg(feature = "decompile")]
#[test]
fn unpacking_comprehensions_use_the_target_iteration_order() {
    check_fixture_on(
        "def f(rows, maps):\n    return [*row for row in rows if row], {*row for row in rows}, {**m for group in maps for m in group}\ndef gen(rows):\n    return (*row for row in rows if row)\n",
        "result = f([[1,2], [], [3,2]], [[{'a':1}, {'b':2}], [{'a':3}]]), list(gen([[], [4,5], [6]]))",
        &[serc::PythonVersion::Py315],
    );
}

#[cfg(feature = "decompile")]
#[test]
fn consecutive_managers_keep_separate_exception_paths() {
    check_fixture(
        "def run(manager, emit):\n    with manager(1) as a, manager(2) as b:\n        emit(a, b)\n    emit('between')\n    with manager(3) as c:\n        emit(c)\n    emit('done')\n    return 17\n",
        r"
def scenario(fail, suppress):
    events = []
    class Manager:
        def __init__(self, n):
            self.n = n
        def __enter__(self):
            events.append(('enter', self.n))
            return self.n
        def __exit__(self, typ, value, tb):
            events.append(('exit', self.n, typ.__name__ if typ else None))
            return self.n == suppress
    def emit(*values):
        events.append(values)
        if values[0] == fail:
            raise ValueError()
    try:
        value = run(Manager, emit)
    except ValueError:
        value = 'raised'
    return value, events
result = [scenario(*args) for args in [(0,0), (1,0), (1,1), (3,3), ('between',3)]]",
    );
}

#[cfg(feature = "decompile")]
#[test]
fn forward_joins_and_loop_exits() {
    check_fixture(
        "def first(xs, emit):\n    for x in xs:\n        emit(x)\n        break\n    else:\n        emit(0)\n    emit(9)\n",
        "result = []\nfirst([], result.append)\nfirst([1, 2], result.append)",
    );
    check_fixture(
        "def f(a, b, c, log):\n    if (a() and b()) or c():\n        log(1)\n    return 2\n",
        "from itertools import product\nresult = []\nfor a, b, c in product([False, True], repeat=3):\n    def probe(n, v): result.append(n); return v\n    result.append(f(lambda: probe('a', a), lambda: probe('b', b), lambda: probe('c', c), result.append))",
    );
    check_fixture(
        "def f(xs, emit):\n    for x in xs:\n        if x < 0:\n            continue\n        if x == 2:\n            break\n        emit(x)\n    else:\n        emit(0)\n    return 1\n",
        "result = []\nfor xs in [[], [-1, 1], [1, 2, 3]]: result.append(f(xs, result.append))",
    );
    check_fixture(
        "def f(pred, stop, emit):\n    while pred():\n        if stop():\n            break\n        emit(1)\n    else:\n        emit(2)\n    return 3\n",
        "result = []\nfor stop in [True, False]:\n    pred = iter([True, True, False])\n    result.append(f(lambda: next(pred), lambda: stop, result.append))",
    );
}

#[cfg(feature = "decompile")]
#[test]
fn exception_edges_and_loop_cleanup() {
    check_fixture(
        "def f(xs, emit):\n    for x in xs:\n        try:\n            if x:\n                break\n        finally:\n            emit(x)\n    return 2\n",
        "result = []\nresult.append(f([0, 1, 2], result.append))\nclass Bad:\n    def __bool__(self): raise ValueError\ntry: f([Bad()], lambda x: result.append('cleanup'))\nexcept ValueError: result.append('caught')",
    );
    check_fixture(
        "def f(work, emit):\n    try:\n        return work()\n    except ValueError:\n        emit(1)\n    except TypeError:\n        emit(2)\n    return 3\n",
        "result = []\ndef fail(e): raise e\nfor work in [lambda: 0, lambda: fail(ValueError()), lambda: fail(TypeError()), lambda: fail(KeyError())]:\n    try: result.append(f(work, result.append))\n    except KeyError: result.append('unmatched')",
    );
}
