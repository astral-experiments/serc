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
fn strict_mode_requires_complete_modules() {
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
                    .arg("--strict")
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
    check_output(source, exercise, versions, |_| {});
}

#[cfg(feature = "decompile")]
fn check_output(
    source: &str,
    exercise: &str,
    versions: &[serc::PythonVersion],
    inspect: impl Fn(&str),
) {
    check_output_at(source, exercise, versions, 0, inspect);
}

#[cfg(feature = "decompile")]
fn check_output_at(
    source: &str,
    exercise: &str,
    versions: &[serc::PythonVersion],
    optimize: u8,
    inspect: impl Fn(&str),
) {
    let temp = tempfile::tempdir().unwrap();
    let original = temp.path().join("original.py");
    let recovered = temp.path().join("recovered.py");
    let cache = temp.path().join("file.pyc");
    fs_err::write(&original, source).unwrap();
    for &version in versions {
        let built = common::python_command(version).args(["-c", "import py_compile, sys; py_compile.compile(sys.argv[1], sys.argv[2], optimize=int(sys.argv[3]), doraise=True)"]).arg(&original).arg(&cache).arg(optimize.to_string()).output().unwrap();
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
        inspect(std::str::from_utf8(&output.stdout).unwrap());
        fs_err::write(&recovered, output.stdout).unwrap();
        let oracle = common::python_command(version)
            .args([
                "-c",
                r"
import sys, types
def load(path):
    return compile(open(path, encoding='utf-8').read(), '<fixture>', 'exec', optimize=int(sys.argv[4]))
def fields(c):
    return (c.co_code, c.co_flags, c.co_names, c.co_varnames, c.co_cellvars, c.co_freevars,
            c.co_argcount, c.co_posonlyargcount, c.co_kwonlyargcount, c.co_stacksize, c.co_exceptiontable,
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
            .arg(optimize.to_string())
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
fn formatted_displays_remain_interpolations() {
    check_fixture(
        "def show(x):\n    return f\"{ {'a': x}!r}\", f\"nested { {f'{ {x} }'} } end\"\n",
        "result = show(42)",
    );
    check_fixture(
        "def fields(xs):\n    return {f':{x}' for x in xs}, {f'{x}': f'{x!r}' for x in xs}\n",
        "result = fields(['a', 2])",
    );
}

#[cfg(feature = "decompile")]
#[test]
fn a_backedge_retests_the_owning_while_condition() {
    check_fixture(
        r"def wake(waiters):
    while waiters:
        waiter = waiters.popleft()
        if not waiter.done():
            waiter.set_result(None)
            break
",
        r"from collections import deque
result = []
class Waiter:
    def __init__(self, value):
        self.value = value
        self.checked = False
    def done(self):
        assert not self.checked, 'retested the previous waiter'
        self.checked = True
        result.append(self.value)
        return self.value
    def set_result(self, value): result.append('woke')
for values in ([], [True, True], [True, False, False]):
    waiters = deque(map(Waiter, values))
    wake(waiters)
    result.append(len(waiters))
assert result == [0, True, True, 0, True, False, 'woke', 1]",
    );
}

#[cfg(feature = "decompile")]
#[test]
fn the_last_while_arm_can_fall_out_after_earlier_backedges() {
    check_fixture(
        r"def walk(node, edge):
    while node is not None:
        if edge(node, 'left'):
            node = node.left
        elif edge(node, 'right'):
            node = node.right
        else:
            break
    return node
",
        r"class Node: pass
a, b, leaf = Node(), Node(), Node()
a.left, b.right = b, leaf
calls = []
def edge(node, name):
    calls.append(name)
    assert len(calls) <= 5, 'revisited the terminal node'
    return hasattr(node, name)
result = walk(a, edge) is leaf, calls
assert result == (True, ['left', 'left', 'right', 'left', 'right'])",
    );
}

#[cfg(feature = "decompile")]
#[test]
fn while_predicate_can_span_the_inlined_any_call() {
    check_fixture_on(
        r"def drain(rows):
    while any(row for row in rows):
        for row in rows:
            if row:
                row.pop()
    return rows
",
        "result = [drain([]), drain([[1, 2], [], [3]])]",
        &[serc::PythonVersion::Py314, serc::PythonVersion::Py315],
    );
}

#[cfg(feature = "decompile")]
#[test]
fn copied_while_conditions_can_suspend_or_receive_a_continue() {
    check_fixture(
        r"def poll(check, events):
    while (yield from check()):
        events.append('ok')

def drain(values, events):
    while values:
        if values.pop():
            continue
        events.append('zero')
",
        r"result = []
answers = iter([True, True, False])
def check():
    yield 'poll'
    return next(answers)
result += list(poll(check, result))
drain([0, 1, 0], result)
assert result == ['ok', 'ok', 'poll', 'poll', 'poll', 'zero', 'zero']",
    );
}

#[cfg(feature = "decompile")]
#[test]
fn a_nested_while_exhausts_into_the_enclosing_for() {
    check_fixture(
        r"def drain(counts, emit):
    for n in counts:
        while n > 0:
            emit(n)
            n -= 1
",
        r"result = []
def emit(n):
    result.append(n)
    assert len(result) <= 3
drain([2, 0, 1], emit)
assert result == [2, 1, 1]",
    );
}

#[cfg(feature = "decompile")]
#[test]
fn a_trailing_break_test_repeats_the_whole_loop_body() {
    check_fixture(
        r"def drain(step, log):
    while True:
        n = step()
        if not n:
            break
    log.append(n)
    return n
",
        r"items = iter([2, 1, 0, 'unused'])
log = []
result = drain(items.__next__, log), next(items), log
assert result == (0, 'unused', [0])",
    );
}

#[cfg(feature = "decompile")]
#[test]
fn a_successful_match_can_skip_the_remaining_cases() {
    check_fixture(
        r"def resolve(name, ready, log):
    match name:
        case 'a':
            return 1
        case 'b':
            log.append('b')
            if ready:
                return 2
        case 'b':
            return 'retried'
        case 'c':
            if ready:
                return 3
    raise KeyError(name)
",
        r"result = []
for name in 'abcd':
    for ready in [False, True]:
        try:
            result.append(resolve(name, ready, result))
        except KeyError as e:
            result.append(e.args)
assert result == [1, 1, 'b', ('b',), 'b', 2, ('c',), 3, ('d',), ('d',)]",
    );
}

#[cfg(feature = "decompile")]
#[test]
fn an_early_return_cleans_up_the_iterator_after_evaluating_its_value() {
    check_fixture(
        r"def ready(items, accept):
    for item in items:
        if accept(item):
            return True
    return False

def first(items, accept, convert):
    for item in items:
        if accept(item):
            return convert(item)
    return False
",
        r"result = []
class Items:
    def __iter__(self):
        try:
            for value in range(3):
                result.append(value)
                yield value
        finally:
            result.append('closed')
def convert(value):
    result.append('convert')
    return value + 10
assert first(Items(), lambda v: v == 1, convert) == 11
assert ready(Items(), lambda v: v == 0)
assert not ready([], bool)
assert result == [0, 1, 'convert', 'closed', 0, 'closed']",
    );
}

#[cfg(feature = "decompile")]
#[test]
fn a_terminal_module_guard_keeps_the_raise_in_the_else() {
    for side in ["little", "big"] {
        let source = format!(
            "side = '{side}'\nif side == 'little':\n    value = 1\nelif side == 'big':\n    value = 2\nelse:\n    raise RuntimeError(side)\n"
        );
        check_fixture(
            &source,
            "result = value\nassert value == (1 if side == 'little' else 2)",
        );
    }
}

#[cfg(feature = "decompile")]
#[test]
fn a_guard_and_loop_exhaustion_can_return_the_same_constant() {
    check_fixture(
        r"def wake(waiters, emit):
    if not waiters:
        return False
    for waiter in waiters:
        if not waiter:
            emit(waiter)
            return True
    return False
",
        r"result = []
assert not wake([], result.append)
assert not wake([1], result.append)
assert wake([1, 0, 0], result.append)
assert result == [0]",
    );
}

#[cfg(feature = "decompile")]
#[test]
fn conditional_none_value_is_distinct_from_a_guard_return() {
    check_fixture(
        r"def keep(reset, value):
    return None if reset else value
",
        "result = [keep(True, 1), keep(False, 2)]",
    );
    check_fixture_on(
        "def pair(a, b):\n    return None if a is None else (a, b)\n",
        "result = [pair(None, 2), pair(1, 2)]",
        &[serc::PythonVersion::Py312, serc::PythonVersion::Py313],
    );
}

#[cfg(feature = "decompile")]
#[test]
fn a_nested_guard_can_return_before_a_shared_continuation() {
    check_fixture(
        r"def finish(future):
    if not future.cancelled():
        exc = future.exception()
        if isinstance(exc, (SystemExit, KeyboardInterrupt)):
            return
    future.stop()
",
        r"result = []
class Future:
    def __init__(self, cancelled, exc):
        self.cancel = cancelled
        self.exc = exc
    def cancelled(self): return self.cancel
    def exception(self):
        assert not self.cancel
        result.append('checked')
        return self.exc
    def stop(self): result.append('stopped')
for cancelled, exc in [(True, None), (False, None), (False, SystemExit())]:
    finish(Future(cancelled, exc))
assert result == ['stopped', 'checked', 'stopped', 'checked']",
    );
}

#[cfg(feature = "decompile")]
#[test]
fn nested_guards_can_each_own_a_copy_of_the_function_exit() {
    check_fixture(
        r"def wake(waiter, events):
    if waiter is not None:
        events.append('waiting')
        if not waiter():
            events.append('woke')
",
        r"result = []
for waiter in [None, lambda: True, lambda: False]:
    wake(waiter, result)
assert result == ['waiting', 'waiting', 'woke']",
    );
}

#[cfg(feature = "decompile")]
#[test]
fn conditional_guards_keep_distinct_function_exits() {
    check_fixture(
        r"def emit(ready, allowed, events):
    if ready and allowed():
        events.append('done')

def guard(skip, ready, events):
    if skip:
        return
    if ready:
        events.append('ran')
",
        r"result = []
def check():
    result.append('checked')
    return True
emit(False, check, result)
emit(True, check, result)
guard(True, True, result)
guard(False, False, result)
guard(False, True, result)
assert result == ['checked', 'done', 'ran']",
    );
}

#[cfg(feature = "decompile")]
#[test]
fn nested_conditional_arms_share_the_outer_jump() {
    check_fixture(
        "def limits(high, low, call):\n    if high is None:\n        if low is None:\n            hi = call(1)\n        else:\n            lo = low\n            hi = call(4) * lo\n    else:\n        hi = high\n    return hi + call(2)\n",
        "log = []\ndef observed(x): log.append(x); return x\nresult = [limits(h, l, observed) for h, l in [(None, None), (None, 3), (8, None), (8, 3)]], log",
    );
    check_fixture(
        "def validate(hi, lo, bound):\n    if not hi >= lo >= bound():\n        raise ValueError(lo)\n    return hi, lo\n",
        "calls = []\ndef bound(): calls.append(1); return 0\nvalues = []\nfor h, l in [(4, 3), (1, 2), (2, -1)]:\n    try: values.append(validate(h, l, bound))\n    except ValueError as e: values.append(e.args)\nresult = values, calls",
    );
}

#[cfg(feature = "decompile")]
#[test]
fn optimized_generator_calls_keep_short_circuiting_and_shadowing() {
    check_fixture(
        "def both(xs, log):\n    a = all(log(x) for x in xs)\n    b = any(log(x) for x in xs)\n    return a, b\ndef first(xs): return any(x for x in xs)\n",
        "seen = []\ndef observe(x): seen.append(x); return x\nresult = [both([1, 0, 2], observe), first([]), seen.copy()]\nall = any = tuple\nresult += [both([0, 2], observe), first([3, 4]), seen.copy()]",
    );
}

#[cfg(feature = "decompile")]
#[test]
fn private_declarations_keep_their_unmangled_names() {
    check_output(
        "class _Outer:\n    def __method(self, x):\n        def __local(): return x\n        return __local\n    class __Inner:\n        def __method(self): return 7\n    type __Alias = int\n",
        "f = _Outer()._Outer__method(42)\ninner = _Outer._Outer__Inner\nalias = _Outer._Outer__Alias\nresult = f(), f.__qualname__, inner.__qualname__, inner()._Inner__method(), alias.__name__, alias.__value__",
        &common::PYTHON_VERSIONS,
        |source| {
            assert!(source.contains("def __method("), "{source}");
            assert!(source.contains("class __Inner"), "{source}");
            assert!(source.contains("type __Alias ="), "{source}");
        },
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
fn raising_handlers_end_before_exception_name_cleanup() {
    check_output(
        r"def named(action):
    try: action()
    except ValueError as error:
        raise RuntimeError('wrapped') from error
    return 42
def unnamed(action):
    try: action()
    except ValueError: raise
    return 43
",
        r"def value(): raise ValueError('v')
def key(): raise KeyError('k')
result = []
for f in [named, unnamed]:
    for action in [lambda: None, value, key]:
        try: result.append(f(action))
        except Exception as exc: result.append((type(exc).__name__, repr(exc.__cause__)))",
        &common::PYTHON_VERSIONS,
        |source| assert!(!source.starts_with("# serc: best-effort"), "{source}"),
    );
}

#[cfg(feature = "decompile")]
#[test]
fn incremental_dictionaries_preserve_order_across_chunks() {
    // CPython inserts 16- and 17-pair runs individually. Larger displays
    // merge multiple such runs, and unpackings can also split a run.
    for size in [15, 16, 17, 18, 35] {
        let entries = (0..size)
            .map(|i| format!("key({i}): value({i})"))
            .collect::<Vec<_>>()
            .join(", ");
        check_fixture(
            &format!(
                "def f(key, value, extra):\n    return {{{entries}, **extra, key(0): value(42)}}\n"
            ),
            "events = []\ndef key(i): events.append(('key', i)); return i % 7\ndef value(i): events.append(('value', i)); return i\nresult = list(f(key, value, {7: 'extra'}).items()), events",
        );
    }
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

#[cfg(feature = "decompile")]
#[test]
fn decorated_classes_and_metaclasses() {
    check_fixture(
        r#"events = []
def mark(which):
    events.append(which)
    def decorate(cls):
        events.append(cls.__name__ + which)
        return cls
    return decorate

class Base:
    pass

@mark("outer")
@mark("inner")
class Derived(Base, metaclass=type):
    pass
"#,
        "result = events, Derived.__base__.__name__",
    );
}

#[cfg(feature = "decompile")]
#[test]
fn complex_constants_and_multidimensional_slices() {
    check_fixture(
        r"values = (12.25j, -3j, 2+3j, -0j, 2-0j, 1e999, -1e999, 1e999j)
class Echo:
    def __getitem__(self, key):
        return key
def keys(x, n):
    return x[3, 1:4], x[n:, :n:2], x[n:,], x[:,]
",
        "result = repr(values), keys(Echo(), 2)",
    );
}

#[cfg(feature = "decompile")]
#[test]
fn lazy_type_aliases_and_bounds() {
    check_fixture(
        r"type Empty = None
type Forward = Later
type Pair[T: (str, bytes), U: T] = tuple[T, U]
class Later:
    type Element = int
",
        "result = Forward.__value__.__name__, Pair.__value__.__args__, Later.Element.__value__; result = repr(result)",
    );
    check_fixture_on(
        "type Params[T: int = int, *Ts = *tuple[str, ...]] = T\n",
        "result = repr(Params.__type_params__[0].__default__)",
        &common::PYTHON_VERSIONS[1..],
    );
}

#[cfg(feature = "decompile")]
#[test]
fn generic_classes_and_super() {
    check_fixture(
        r#"class Base[T]:
    def method(self):
        return "base"
def record(cls):
    return cls
@record
class Child[T: int](Base[T], metaclass=type):
    value: T
    def method(self):
        return super().method() + super(Child, self).method()
"#,
        "result = Child().method(), repr(Child.__orig_bases__), repr(Child.__annotations__)",
    );
}

#[cfg(feature = "decompile")]
#[test]
fn parallel_assignments_evaluate_before_storing() {
    check_fixture(
        r"events = []
def log(x):
    events.append(x)
    return x
a, b, c = log(1), log(2), log(3)
a, a = b, c
def rotate(x, y, z):
    x, y, z = log(z), log(x), log(y)
    x, y = y, x
    return x, y, z
",
        "result = a, rotate(4, 5, 6), events",
    );
    check_fixture(
        "def fill(box, target, key, value):\n    box.a, box.b, box.c = value(1), value(2), value(3)\n    target()[key()], target().a = value(4), value(5)\n",
        r"class Box(dict): pass
box = Box()
events = []
def target():
    events.append('target')
    return box
def key():
    events.append('key')
    return 6
def value(x):
    events.append(x)
    return x
fill(box, target, key, value)
result = box.a, box.b, box.c, box[6], events
assert result == (5, 2, 3, 4, [1, 2, 3, 4, 5, 'target', 'key', 'target'])",
    );
}

#[cfg(feature = "decompile")]
#[test]
fn comparison_chains_short_circuit() {
    check_fixture(
        r"events = []
def log(value):
    events.append(value)
    return value
def chain(a, b, c):
    return log(a) < log(b) <= log(c)
def branch(a, b, c):
    if log(a) < log(b) < log(c):
        return a
    return c
def mixed(a, b, c):
    return [a is b is not c, a not in c == b]
",
        "result = chain(3, 2, 1), chain(1, 2, 2), branch(3, 2, 1), branch(1, 2, 3), mixed(1, [0], [0]), events",
    );
}

#[cfg(feature = "decompile")]
#[test]
fn value_patterns_and_guards() {
    check_fixture(
        r#"events = []
class Kind:
    key = 17
def test(value):
    events.append(value)
    return value
def classify(value):
    match test(value):
        case Kind.key if test(True): return "key"
        case False: return "false"
        case None: return "none"
        case other: return other
def group(value):
    match value:
        case 0 | 1: return "small"
        case 3 | None | 9: return "other"
def bind(value):
    match value:
        case 0 as zero: return zero
        case 3 as three if three: return three
        case rest: return rest
"#,
        "result = [classify(x) for x in [17, 0, False, None, 'z']], [bind(x) for x in [0, 3, 4]], [group(x) for x in [0, 1, 3, None, 9, 2]], events",
    );
}

#[cfg(feature = "decompile")]
#[test]
fn class_layout_matches_each_occurrence() {
    check_fixture(
        r"class C: pass
first = C
class C: pass
def item(): return 3

def decorate(cls): return cls
@decorate
class C:
    class C: pass
    def get(self): return item()
class Last: pass
",
        "result = first.__name__, C.C.__qualname__, C().get(), Last.__name__",
    );
}

#[cfg(feature = "decompile")]
#[test]
fn handlers_survive_an_elided_try_suite() {
    check_fixture(
        r"events = []
try:
    pass
except ValueError as error:
    events.append(str(error))
except TypeError:
    events.append(2)
try:
    pass
except:
    events.append(3)
events.append(4)
",
        "result = events",
    );
}

#[cfg(feature = "decompile")]
#[test]
fn async_comprehension_clause_order() {
    check_fixture(
        r"async def items(rows):
    for row in rows:
        yield row
async def consume():
    rows = [[1, 0], [2, 3]]
    left = [v async for row in items(rows) for v in row if v]
    right = {v: v+1 for row in rows async for v in items(row) if v}
    stream = (v async for row in items(rows) async for v in items(row) if v)
    return left, right, [v async for v in stream]
",
        "import asyncio; result = asyncio.run(consume())",
    );
}

#[cfg(feature = "decompile")]
#[test]
fn synthetic_dead_code_is_marked_at_each_site() {
    check_output(
        "if False: hidden()\nanswer = 1\nif False: later()\n",
        "result = answer",
        &common::PYTHON_VERSIONS,
        |source| {
            assert_eq!(
                source,
                "if False: hidden  # serc: synthetic dead code\nanswer = 1\nif False: later  # serc: synthetic dead code\n"
            );
        },
    );
    check_output(
        "def unchanged(arg):\n    return arg\ndef f():\n    if False: lost = hidden()\n    return lost\n",
        "try: f()\nexcept UnboundLocalError: result = unchanged(42)",
        &common::PYTHON_VERSIONS,
        |source| {
            assert_eq!(source.matches("# serc: synthetic dead code").count(), 1);
            assert!(source.contains("lost = hidden  # serc: synthetic dead code"));
        },
    );
    check_output(
        "if 0: pass\nanswer = 1\n",
        "result = answer",
        &common::PYTHON_VERSIONS,
        |source| assert!(source.contains("if 0: pass  # serc: synthetic dead code")),
    );
}

#[cfg(feature = "decompile")]
#[test]
fn synthetic_dead_code_still_requires_whole_module_verification() {
    let temp = tempfile::tempdir().unwrap();
    let cache = temp.path().join("altered.pyc");
    for version in common::PYTHON_VERSIONS {
        let built = common::python_command(version)
            .args([
                "-c",
                r"
import importlib.util, marshal, sys
from pathlib import Path
module = compile('if False: hidden()\ndef f(): return 1\n', 'lost.py', 'exec')
constants = tuple(c.replace(co_stacksize=c.co_stacksize + 1) if hasattr(c, 'co_stacksize')
                  else c for c in module.co_consts)
Path(sys.argv[1]).write_bytes(importlib.util.MAGIC_NUMBER + bytes(12)
                              + marshal.dumps(module.replace(co_consts=constants)))",
            ])
            .arg(&cache)
            .output()
            .unwrap();
        assert!(built.status.success(), "{built:?}");
        let output = Command::new(env!("CARGO_BIN_EXE_serc"))
            .arg("decompile")
            .arg("--strict")
            .arg(&cache)
            .output()
            .unwrap();
        assert!(!output.status.success(), "{version}");
        assert!(output.stdout.is_empty());
    }
}

#[cfg(feature = "decompile")]
#[test]
fn comprehensions_inside_returning_exception_handlers() {
    check_fixture(
        "def exports(module):\n    try:\n        return list(module.__all__)\n    except AttributeError:\n        return [n for n in dir(module) if n[0] != '_']\n",
        "class Explicit: __all__ = ('_keep',)\nclass Implicit: visible = 1\nresult = exports(Explicit), exports(Implicit)\nassert result == (['_keep'], ['visible'])",
    );
    check_fixture(
        "def values(items, work):\n    before = [x + 1 for x in items]\n    try: work()\n    except KeyError: before.append(5)\n    return before\n",
        "def fail(): raise KeyError\nresult = values([1], fail), values([3], lambda: None)\nassert result == ([2,5],[4])",
    );
}

#[cfg(feature = "decompile")]
#[test]
fn suspended_handlers_resume_and_reraise_in_loops() {
    check_fixture(
        r"async def consume(pending, wait, events):
    while pending:
        waiter = pending.pop()
        try:
            await wait(waiter)
        except:
            events.append('cancel')
            try:
                pending.remove(waiter)
            except ValueError:
                pass
            if pending and not waiter:
                events.append('wake')
            raise
    return 7
",
        r"import asyncio
result = []
async def ok(waiter): pass
async def cancel(waiter): raise asyncio.CancelledError
result.append(asyncio.run(consume([1], ok, result)))
for pending in ([1, 0], [0, 0]):
    try: asyncio.run(consume(pending, cancel, result))
    except asyncio.CancelledError: result.append(list(pending))
assert result == [7, 'cancel', 'wake', [1], 'cancel', []]",
    );
    check_fixture(
        r"async def read(work, events):
    try: await work()
    except KeyError:
        events.append('caught')
        raise
    return 7
",
        r"import asyncio
result = []
async def run(value):
    async def task():
        if value: raise value
    try: result.append(await read(task, result))
    except (KeyError, ValueError) as exc: result.append(type(exc).__name__)
for value in (None, KeyError(), ValueError()): asyncio.run(run(value))
assert result == [7, 'caught', 'KeyError', 'ValueError']",
    );
    check_fixture(
        r"async def read(work):
    try:
        await work(1)
        return await work(2)
    except KeyError:
        raise
",
        r"import asyncio
result = []
async def work(n):
    result.append(n)
    return n + 40
result.append(asyncio.run(read(work)))
assert result == [1, 2, 42]",
    );
}

#[cfg(feature = "decompile")]
#[test]
fn finally_runs_on_suspension_return_and_cancellation() {
    check_fixture(
        r"async def value(work, events):
    try:
        return await work()
    finally:
        events.append('cleanup')

async def constant(work, events):
    try:
        await work()
        return True
    finally:
        events.append('cleanup')

async def resume(work, events):
    try:
        await work()
    finally:
        events.append('cleanup')
    events.append('resumed')
",
        r"import asyncio
result = []
async def ok(): return 42
async def cancel(): raise asyncio.CancelledError
for run in (value, constant, resume):
    result.append(asyncio.run(run(ok, result)))
    try: asyncio.run(run(cancel, result))
    except asyncio.CancelledError: result.append('cancelled')
assert result == ['cleanup', 42, 'cleanup', 'cancelled',
                  'cleanup', True, 'cleanup', 'cancelled',
                  'cleanup', 'resumed', None, 'cleanup', 'cancelled']",
    );
}

#[cfg(feature = "decompile")]
#[test]
fn guarded_tries_keep_handlers_in_their_original_branches() {
    check_fixture(
        r"def hooks(first, second):
    if first:
        try:
            first()
        except ValueError:
            pass
    if second:
        try:
            second()
        except ValueError:
            pass
",
        r"result = []
def fail():
    result.append(1)
    raise ValueError
def other(): raise KeyError
hooks(fail, lambda: result.append(2))
hooks(None, fail)
try: hooks(other, lambda: result.append('skipped'))
except KeyError: result.append(3)
assert result == [1, 2, 1, 3]",
    );
    check_fixture(
        r"def read(hook, ready, work, finish):
    try:
        if hook:
            hook()
        while ready():
            work()
        return ready()
    finally:
        finish()
",
        r"result = []
def work(): result.append('work')
def done(): result.append('finished')
def fail(): raise ValueError
assert read(None, lambda: not result, work, done) is False
try: read(fail, lambda: True, work, done)
except ValueError: result.append('raised')
assert result == ['work', 'finished', 'finished', 'raised']",
    );
}

#[cfg(feature = "decompile")]
#[test]
fn ordered_exception_clauses_keep_the_success_path() {
    check_fixture(
        r"def receive(read, events):
    try:
        data = read()
    except (BlockingIOError, InterruptedError):
        return
    except (SystemExit, KeyboardInterrupt):
        raise
    except BaseException as exc:
        events.append(type(exc).__name__)
    else:
        events.append(data)
",
        r"result = []
def fail(exc):
    def read(): raise exc
    return read
receive(lambda: 42, result)
for exc in (BlockingIOError(), InterruptedError(), SystemExit(), KeyboardInterrupt(), ValueError()):
    try: receive(fail(exc), result)
    except (SystemExit, KeyboardInterrupt): result.append('raised')
assert result == [42, 'raised', 'raised', 'ValueError']",
    );
    check_fixture(
        r"def run(work, events):
    try:
        value = work()
    except KeyError as exc:
        events.append(str(exc))
    except:
        return 7
    else:
        events.append(value)
    events.append('continued')
    return 9
",
        r"result = []
def missing(): raise KeyError(1)
def other(): raise ValueError
for task in (lambda: 42, missing, other): result.append(run(task, result))
assert result == [42, 'continued', 9, '1', 'continued', 9, 7]",
    );
}

#[cfg(feature = "decompile")]
#[test]
fn mismatched_exception_tables_mark_only_the_affected_function() {
    let temp = tempfile::tempdir().unwrap();
    let original = temp.path().join("original.py");
    let cache = temp.path().join("module.pyc");
    let recovered = temp.path().join("recovered.py");
    fs_err::write(
        &original,
        r"def keep(fn): return fn
@keep
async def reader(work, events):
    try: await work()
    except KeyError:
        events.append('lost')
        raise
    return 7
first = reader
def reader(x): return x + 1
",
    )
    .unwrap();
    for version in common::PYTHON_VERSIONS {
        let built = common::python_command(version)
            .args(["-c", r"import dis, importlib.util, marshal, sys, types
from pathlib import Path
code = compile(Path(sys.argv[1]).read_bytes(), '<fixture>', 'exec')
constants = list(code.co_consts)
i = next(i for i, c in enumerate(constants) if isinstance(c, types.CodeType) and c.co_name == 'reader')
reader = constants[i]
# Split the final unwind range at a real instruction. The two entries still
# protect exactly the same operations, but the compiler emits a single range.
entries = list(dis.Bytecode(reader).exception_entries)
last = entries.pop()
split = next(op.offset for op in dis.get_instructions(reader) if last.start < op.offset < last.end)
entries.extend([last._replace(end=split), last._replace(start=split)])
def varint(n):
    data = [n & 63]
    while n >= 64:
        n >>= 6
        data.append(64 | (n & 63))
    return bytes(reversed(data))
table = bytearray()
for start, end, target, depth, lasti in entries:
    at = len(table)
    table += varint(start // 2) + varint((end-start) // 2) + varint(target // 2) + varint(depth*2 + lasti)
    table[at] |= 128
constants[i] = reader.replace(co_exceptiontable=bytes(table))
Path(sys.argv[2]).write_bytes(importlib.util.MAGIC_NUMBER + bytes(12) + marshal.dumps(code.replace(co_consts=tuple(constants))))
"])
            .arg(&original).arg(&cache).output().unwrap();
        assert!(built.status.success(), "{built:?}");
        let output = Command::new(env!("CARGO_BIN_EXE_serc"))
            .arg("decompile")
            .arg(&cache)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        let source = String::from_utf8(output.stdout).unwrap();
        let header =
            "async def reader(work, events):  # serc: exception table differs after recompilation";
        assert!(source.contains(header), "{source}");
        assert_eq!(
            source
                .matches("# serc: exception table differs after recompilation")
                .count(),
            1
        );
        assert_eq!(
            source
                .matches("# serc: original blocks (exception table differs) in reader ")
                .count(),
            1
        );
        fs_err::write(&recovered, source).unwrap();
        let checked = common::python_command(version).args(["-c", r"import asyncio, dis, marshal, re, runpy, sys, types
with open(sys.argv[1], 'rb') as f:
    f.read(16)
    original = next(c for c in marshal.load(f).co_consts if isinstance(c, types.CodeType) and c.co_name == 'reader')
text = open(sys.argv[2], encoding='utf8').read().split('# serc: original blocks (exception table differs) in reader ')[1]
assert [(int(m[1]),m[2]) for m in re.finditer(r'^# +(\d+)  (\w+) ', text, re.M)] == [(op.offset,op.opname) for op in dis.get_instructions(original)]
scope = runpy.run_path(sys.argv[2])
async def ok(): return None
assert asyncio.run(scope['first'](ok, [])) == 7 and scope['reader'](3) == 4
"]).arg(&cache).arg(&recovered).output().unwrap();
        assert!(checked.status.success(), "{checked:?}");
    }
}

#[cfg(feature = "decompile")]
#[test]
fn normal_flow_warnings_list_the_original_successors() {
    let temp = tempfile::tempdir().unwrap();
    let cache = temp.path().join("guard.pyc");
    let recovered = temp.path().join("guard.py");
    for version in common::PYTHON_VERSIONS {
        let built = common::python_command(version).args(["-c", r"import dis, importlib.util, marshal, sys, types
from pathlib import Path
code = compile('def keep(x): return x\ndef route(a, b, effect):\n    if a and b: effect()\n', 'guard.py', 'exec')
constants = list(code.co_consts)
index = next(i for i, c in enumerate(constants) if isinstance(c, types.CodeType) and c.co_name == 'route')
route = constants[index]
# Share the two false exits, leaving the second copy of the return in place.
first, second = [op for op in dis.get_instructions(route) if op.opname == 'POP_JUMP_IF_FALSE']
assert first.argval != second.argval
ops = bytearray(route.co_code)
ops[first.offset + 1] += (second.argval - first.argval) // 2
constants[index] = route.replace(co_code=bytes(ops))
Path(sys.argv[1]).write_bytes(importlib.util.MAGIC_NUMBER + bytes(12) + marshal.dumps(code.replace(co_consts=tuple(constants))))"])
            .arg(&cache).output().unwrap();
        assert!(built.status.success(), "{built:?}");
        let output = Command::new(env!("CARGO_BIN_EXE_serc"))
            .arg("decompile")
            .arg(&cache)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        let source = String::from_utf8(output.stdout).unwrap();
        assert!(
            source.contains("def route(a, b, effect):  # serc: normal control flow differs"),
            "{source}"
        );
        assert_eq!(
            source
                .matches("# serc: normal control flow differs")
                .count(),
            1
        );
        assert_eq!(source.matches("# serc: original blocks").count(), 1);
        fs_err::write(&recovered, source).unwrap();
        let checked = common::python_command(version).args(["-c", r"import dis, marshal, re, runpy, sys, types
from pathlib import Path
route = next(c for c in marshal.loads(Path(sys.argv[1]).read_bytes()[16:]).co_consts if isinstance(c, types.CodeType) and c.co_name == 'route')
listing = Path(sys.argv[2]).read_text().split('# serc: original blocks (normal control flow differs) in route ')[1]
ops = list(dis.get_instructions(route))
assert [(int(m[1]), m[2]) for m in re.finditer(r'^# +(\d+)  (\w+) ', listing, re.M)] == [(op.offset, op.opname) for op in ops]
assert [int(n) for n in re.findall(r'^# block .* jump (\d+)$', listing, re.M)] == [op.argval for op in ops if op.opname == 'POP_JUMP_IF_FALSE']
scope = runpy.run_path(sys.argv[2])
calls = []
for a in [False, True]:
    for b in [False, True]:
        scope['route'](a, b, lambda: calls.append((a, b)))
assert calls == [(True, True)] and scope['keep'](42) == 42"])
            .arg(&cache).arg(&recovered).output().unwrap();
        assert!(checked.status.success(), "{checked:?}");
    }
}

#[cfg(feature = "decompile")]
#[test]
fn partial_modules_keep_siblings_and_mark_unrecovered_bodies() {
    let temp = tempfile::tempdir().unwrap();
    let original = temp.path().join("original.py");
    let cache = temp.path().join("module.pyc");
    let recovered = temp.path().join("recovered.py");
    let source = format!(
        r"before = 7
if before:
    lost = '{big}'
else:
    wrongly_hoisted()
def large():
    return '{big}'
def after(x):
    return x + before
if before:
    lost = '{big}'
    def repeated(x, /):
        def inner():
            return x + 1
        return inner
else:
    async def repeated(*, y):
        return y + 2
if before:
    lost = '{big}'
    class Lost:
        try:
            if before: value = before + 4
        except NameError: value = 0
    hidden = lambda z: z + 3
    def incomplete(): return '{big}'
fake_gap = '# __serc_unplaced__ 0 1 2 # serc: unrecovered literal'
fake_marker = '# __serc_definition_id__ 7'
label: str = 'αβγ'
class Labels:
    label: str = 'αβγ'
    size: int = 3
def annotated(x: int): return x
if before:
    lost = '{big}'
    def __annotate__(user_format): return user_format
def protected(flag, action, events):
    if flag: prefix = '{big}'
    else: wrongly_hoisted()
    try: action()
    except KeyError: events.append('caught')
    tail = '{big}'
    events.append('after')
    return 7
def outer(events):
    def get(): return x
    def put(value):
        nonlocal x
        x = value
    events.append((get, put))
    try:
        x = '{big}'
        return 0
    except Exception:
        return put
def unfinished():
    def kept(): return 3
    try:
        action()
        return '{big}'
    except Exception: return kept
def guarded(flag, events):
    if flag:
        normal = 1
        try: events.append('guarded')
        except KeyError: pass
        events.append('tail')
    else:
        events.append('other')
    return events
",
        big = "x".repeat(200_000)
    );
    fs_err::write(&original, source).unwrap();
    for version in common::PYTHON_VERSIONS {
        let built = common::python_command(version)
            .args(["-c", "import py_compile, sys; py_compile.compile(sys.argv[1], sys.argv[2], doraise=True)"])
            .arg(&original).arg(&cache).output().unwrap();
        assert!(built.status.success(), "{built:?}");
        let output = Command::new(env!("CARGO_BIN_EXE_serc"))
            .arg("decompile")
            .arg(&cache)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(text.starts_with("# serc: best-effort decompilation\n"));
        assert!(text.contains("def large():\n    ...  # serc: unrecovered remainder of large"));
        assert!(text.contains("def after(x):"));
        assert!(!text.contains("# serc: unplaced blocks in after ("));
        assert!(!text.contains("# serc: unplaced blocks in Labels ("));
        assert!(text.contains("# serc: detached function incomplete ("));
        assert!(text.contains("# serc: unplaced blocks in incomplete ("));
        assert!(text.contains("# serc: unrecovered statement in <module>"));
        assert_eq!(
            text.matches("# serc: unrecovered statement in protected")
                .count(),
            2
        );
        assert!(!text.contains("wrongly_hoisted()"));
        assert!(text.contains("if False: x = None  # serc: synthetic dead code"));
        assert!(text.contains("def kept():"));
        assert_eq!(
            text.matches("# serc: detached function repeated (original line ")
                .count(),
            2
        );
        assert!(text.contains("# def repeated(x, /):\n#     def inner():"));
        assert!(text.contains("# async def repeated(*, y):"));
        assert!(!text.contains("# serc: detached function repeated.<locals>.inner"));
        assert!(!text.contains("# serc: detached function after"));
        assert_eq!(
            text.matches("# serc: detached function __annotate__ ")
                .count(),
            1
        );
        assert!(text.contains("# def __annotate__(user_format):"));
        let appendix = text.split_once("\n# serc: detached function ").unwrap().1;
        assert!(
            appendix
                .lines()
                .skip(1)
                .all(|line| line.is_empty() || line.starts_with('#'))
        );
        fs_err::write(&recovered, text).unwrap();
        let checked = common::python_command(version)
            .args([
                "-c",
                r"import dis, marshal, re, runpy, sys, types
scope = runpy.run_path(sys.argv[1])
assert scope['fake_gap'] == '# __serc_unplaced__ 0 1 2 # serc: unrecovered literal'
# Compare these lost scopes independently against the compiling interpreter.
with open(sys.argv[2], 'rb') as cache:
    cache.read(16)
    module = marshal.load(cache)
sections = {}
for section in open(sys.argv[1], encoding='utf8').read().split('\n# serc: unplaced blocks in ')[1:]:
    name, _, _ = section.partition(' (original line ')
    sections.setdefault(name, []).append(section)
for name in ['Lost', '<lambda>']:
    code, = (c for c in module.co_consts if isinstance(c, types.CodeType) and c.co_name == name)
    section, = sections[name]
    lines = section.splitlines()
    rows = [(int(row.split()[1]), row.split()[2]) for row in lines if re.match(r'^# +[0-9]+  ', row)]
    ops = list(dis.get_instructions(code))
    assert rows == [(op.offset, op.opname) for op in ops], (rows, ops)
    if name == 'Lost':
        assert len(rows) > 12
        blocks = {int(row.split()[2]): row for row in lines if row.startswith('# block ')}
        for entry in dis.Bytecode(code).exception_entries:
            assert f'  except {entry.target} (depth {entry.depth}, lasti {str(entry.lasti).lower()})' in blocks[entry.start]
        for i, op in enumerate(ops):
            if op.opname.startswith('POP_JUMP'):
                header = blocks[max(b for b in blocks if b <= op.offset)]
                assert f'  jump {op.argval}' in header and f'  fallthrough {ops[i+1].offset}' in header
assert scope['after'](5) == 12
assert 'repeated' not in scope and 'inner' not in scope
assert scope['fake_marker'] == '# __serc_definition_id__ 7'
annotations = scope['__annotate__'](1) if '__annotate__' in scope else scope['__annotations__']
assert annotations['label'] is str and scope['label'] == 'αβγ'
assert scope['Labels'].__annotations__ == dict(label=str, size=int)
assert scope['Labels'].label == 'αβγ' and scope['Labels'].size == 3
events = []
def fail(): raise KeyError
for flag, action in [(True, fail), (False, lambda: None)]:
    assert scope['protected'](flag, action, events) == 7
assert events == ['caught', 'after', 'after']
events.clear()
scope['outer'](events)
get, put = events[0]
try: get()
except NameError: pass
else: raise AssertionError('the omitted assignment initialized the cell')
put(42)
assert get() == 42
events = []
scope['guarded'](False, events)
assert 'guarded' not in events and 'tail' not in events
events.clear()
scope['guarded'](True, events)
assert 'other' not in events",
            ])
            .arg(&recovered)
            .arg(&cache)
            .output()
            .unwrap();
        assert!(checked.status.success(), "{checked:?}");
    }
    fs_err::write(&cache, b"not a cache").unwrap();
    let rejected = Command::new(env!("CARGO_BIN_EXE_serc"))
        .arg("decompile")
        .arg(&cache)
        .output()
        .unwrap();
    assert!(!rejected.status.success());
    assert!(rejected.stdout.is_empty());
}

#[cfg(feature = "decompile")]
#[test]
fn synthetic_yields_and_captures_preserve_function_kinds() {
    check_output(
        "def outer(x):\n    async def stream():\n        if False: yield x\n    return stream\n",
        "import asyncio, inspect\nstream = outer(42)\nasync def collect(): return [n async for n in stream()]\nresult = inspect.isasyncgenfunction(stream), stream.__closure__[0].cell_contents, asyncio.run(collect())",
        &common::PYTHON_VERSIONS,
        |source| {
            assert_eq!(source.matches("# serc: synthetic dead code").count(), 1);
            assert!(source.contains("(yield x)  # serc: synthetic dead code"));
        },
    );
    check_output(
        "def stream():\n    if False: yield ghost()\n    raise LookupError\n",
        "import inspect\niterator = stream()\ntry: next(iterator)\nexcept LookupError: result = inspect.isgenerator(iterator)",
        &common::PYTHON_VERSIONS,
        |source| assert!(source.contains("(yield ghost)  # serc: synthetic dead code")),
    );
    check_fixture(
        "def stream(stop):\n    yield 1\n    if stop: return\n    raise LookupError\n",
        "a, b = stream(True), stream(False)\nresult = [next(a), next(b), next(a, 2)]\ntry: next(b)\nexcept LookupError: result.append(3)",
    );
}

#[cfg(feature = "decompile")]
#[test]
fn erased_local_annotation_still_binds_nonlocal_writes() {
    check_output(
        "def outer(*args, **kwargs):\n    slot: int\n    def get(): return slot\n    def put(value):\n        nonlocal slot\n        slot = value\n    return get, put\n",
        "get, put = outer()\ntry: get()\nexcept NameError: empty = True\nput(42)\nresult = empty, get()",
        &common::PYTHON_VERSIONS,
        |source| {
            assert!(!source.starts_with("# serc: best-effort"), "{source}");
            assert!(source.contains("if False: slot = None  # serc: synthetic dead code"));
            assert_eq!(source.matches("# serc: synthetic dead code").count(), 1);
        },
    );
}

#[cfg(feature = "decompile")]
#[test]
fn live_guards_are_marked_separately_from_dead_suites() {
    let versions = [serc::PythonVersion::Py314, serc::PythonVersion::Py315];
    for optimize in 0..=2 {
        check_output_at(
            "def stream(arg):\n    if not __debug__: yield arg\n    return 9\nif False: missing()\nanswer = 1\n",
            "it = stream(42)\nitems = list(it)\nresult = items, answer",
            &versions,
            optimize,
            |source| {
                assert!(source.contains("missing  # serc: synthetic dead code"));
                if optimize > 0 {
                    assert!(
                        source.contains("if not False: (yield arg)  # serc: synthetic live guard"),
                        "{source}"
                    );
                } else {
                    assert!(!source.contains("# serc: synthetic live guard"));
                }
            },
        );
    }
}

#[cfg(feature = "decompile")]
#[test]
fn chained_assignments_evaluate_the_rhs_once_and_targets_in_order() {
    check_fixture_on(
        "a: (x := 1)\n",
        "result = x, __annotations__['a']",
        &[serc::PythonVersion::Py312, serc::PythonVersion::Py313],
    );
    check_output(
        "def assign(obj, key, value):\n    first = obj()[key()] = second = obj().attr = value()\n    return first is second\n",
        r"class Obj:
    def __setitem__(self, key, value): log.append(('item', key, value is token))
    def __setattr__(self, key, value): log.append(('attr', key, value is token))
log, token, target = [], object(), Obj()
def obj():
    log.append('obj')
    return target
def key():
    log.append('key')
    return 7
def value():
    log.append('value')
    return token
result = assign(obj, key, value), log",
        &common::PYTHON_VERSIONS,
        |source| assert!(source.contains("first = obj()[key()] = second = obj().attr = value()")),
    );
    check_fixture(
        "def assign(obj, values):\n    a, obj.attr = obj[1], b = values()\n    return a, b\n",
        "class Obj(dict): pass\nobj = Obj()\nresult = assign(obj, lambda: (2, 3)), obj.attr, obj[1]",
    );
}

#[cfg(feature = "decompile")]
#[test]
fn truth_tests_preserve_user_conversions_and_short_circuit_returns() {
    check_fixture(
        "def truth(x):\n    return not x, not not x\ndef choose(read, y):\n    if (x := read()) and y: return x\n    return y\n",
        r"class Flag:
    def __init__(self, name, truth):
        self.name, self.truth = name, truth
    def __bool__(self):
        log.append(self.name)
        return self.truth
log = []
a, b = Flag('a', False), Flag('b', True)
result = truth(a), truth(b), choose(lambda: a, b) is b, choose(lambda: b, a) is a, log",
    );
}

#[cfg(feature = "decompile")]
#[test]
fn managers_resume_after_await_and_preserve_exception_suppression() {
    check_fixture(
        "async def run(m, n, cb):\n    with m as v:\n        await cb(v)\n    async with n as w:\n        await cb(w)\n        await cb(3)\n    return 9\n",
        r"import asyncio
from contextlib import contextmanager, asynccontextmanager
log = []
@contextmanager
def sync():
    log.append('enter')
    try: yield 1
    finally: log.append('exit')
@asynccontextmanager
async def manager(suppress):
    log.append('aenter')
    try: yield 2
    except ValueError:
        log.append('caught')
        if not suppress: raise
    finally: log.append('aexit')
async def cb(value):
    log.append(value)
    await asyncio.sleep(0)
    if value == fail: raise ValueError
answers = []
for fail, suppress in [(0, False), (2, True), (2, False), (1, True)]:
    try: answers.append(asyncio.run(run(sync(), manager(suppress), cb)))
    except ValueError: answers.append('raised')
result = answers, log",
    );
}

#[cfg(feature = "decompile")]
#[test]
fn short_circuit_none_tests_preserve_identity_and_lookup_order() {
    check_fixture(
        "def f(x, read, emit):\n    if x or read() is None: emit(1)\n    if x is None or read() is not None: emit(2)\n    return 3\n",
        r"log = []
class Flag:
    def __bool__(self):
        log.append('tested')
        return True
for x in [False, None, Flag()]:
    for value in [None, Flag()]:
        def read():
            log.append('read')
            return value
        f(x, read, log.append)
result = log",
    );
}
