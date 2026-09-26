use std::ffi::OsString;
use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::sync::OnceLock;

use serc::{
    CompileOptions, OptimizationLevel, PythonVersion, compile, compile_bytes_with_options,
    compile_bytes_with_path_and_options, compile_with_options,
};

const VERSIONS: [(PythonVersion, &str, &str); 4] = [
    (PythonVersion::Py312, "SERC_TEST_PYTHON_312", "python3.12"),
    (PythonVersion::Py313, "SERC_TEST_PYTHON_313", "python3.13"),
    (PythonVersion::Py314, "SERC_TEST_PYTHON", "python3.14"),
    (PythonVersion::Py315, "SERC_TEST_PYTHON_315", "python3.15"),
];
const LEVELS: [OptimizationLevel; 3] = [
    OptimizationLevel::Zero,
    OptimizationLevel::One,
    OptimizationLevel::Two,
];

#[test]
fn optimized_terminal_guards_locate_fallthrough_returns() {
    for source in [
        "def f(x):\n    if x == 1:\n        assert x\n",
        "def f(x):\n    if not x:\n        assert x\n    else:\n        assert not x\n",
        "def f(x):\n    if x:\n        assert x\n    else:\n        pass\n",
        "if x == 1:\n    assert x\n",
        "def f(x):\n    if x == 1:\n        assert x\n    return None\n",
        "def f(x):\n    if x == 1:\n        pass\n",
    ] {
        assert_matches_cpython_versions(
            source,
            "sub/terminal_guard_fallthrough.py",
            &LEVELS,
            &[PythonVersion::Py314],
        );
    }
}

#[test]
fn legacy_nested_finally_pass_keeps_outer_owner() {
    const SOURCE: &str = "def f(work, cleanup, after):\n    try:\n        try:\n            work()\n        finally:\n            pass\n        after()\n    finally:\n        cleanup()\n";
    const TERMINAL: &str = "def f(work, cleanup):\n    try:\n        try:\n            work()\n        finally:\n            pass\n    finally:\n        cleanup()\n";
    const TRACE: &str = r"
import dis, marshal, sys
scope = {}
exec(marshal.loads(sys.stdin.buffer.read()), scope)
f = scope['f']
targets = [instruction.offset for instruction in dis.get_instructions(f)
           if instruction.opname == 'NOP' and instruction.positions.lineno == 6]
if not targets:
    raise RuntimeError('missing pass marker')
target = min(targets)
monitor = sys.monitoring
monitor.use_tool_id(5, 'legacy-finally-owner')
for exception in (ValueError, StopIteration):
    for cleanup_raises in (False, True):
        events, hits = [], []
        def work():
            events.append('work')
        def after():
            events.append('after')
        def cleanup():
            events.append('cleanup')
            if cleanup_raises:
                raise RuntimeError('cleanup')
        def inject(code, offset):
            if offset == target:
                hits.append(offset)
                raise exception('injected')
        monitor.register_callback(5, monitor.events.INSTRUCTION, inject)
        monitor.set_local_events(5, f.__code__, monitor.events.INSTRUCTION)
        try:
            f(work, cleanup, after)
            result = None
        except BaseException as error:
            result = type(error)
        finally:
            monitor.set_local_events(5, f.__code__, 0)
            monitor.register_callback(5, monitor.events.INSTRUCTION, None)
        expected = RuntimeError if cleanup_raises else exception
        if hits != [target] or events != ['work', 'cleanup'] or result is not expected:
            raise RuntimeError((hits, events, result, expected))
monitor.free_tool_id(5)
";

    let versions = [PythonVersion::Py312, PythonVersion::Py313];
    for source in [
        TERMINAL,
        r"def f(work,cleanup,flag):
    try:
        try:
            work()
        finally:
            pass
        global marker
        assert flag
    finally:
        cleanup()
",
        r"def outer():
    marker = 0
    def f(work,cleanup,flag):
        try:
            try:
                work()
            finally:
                pass
            nonlocal marker
            1
        finally:
            cleanup()
    return f
",
    ] {
        assert_matches_cpython_versions(source, "legacy_finally.py", &LEVELS, &versions);
    }
    for (index, python_version) in versions.into_iter().enumerate() {
        for optimization_level in LEVELS {
            let native = oracle(
                SOURCE.as_bytes(),
                "legacy_finally.py",
                python_version,
                optimization_level,
            );
            assert!(
                native.status.success(),
                "{}",
                String::from_utf8_lossy(&native.stderr)
            );
            let actual = compile_with_options(
                SOURCE,
                "legacy_finally.py",
                &CompileOptions {
                    python_version,
                    optimization_level,
                    ..CompileOptions::default()
                },
            )
            .unwrap()
            .marshal();
            assert_eq!(
                actual, native.stdout,
                "{python_version}, {optimization_level:?}"
            );
            for code in [&native.stdout, &actual] {
                let mut child = Command::new(ORACLES[index].get().unwrap())
                    .args(["-I", "-S", "-B", "-c", TRACE])
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .unwrap();
                child.stdin.take().unwrap().write_all(code).unwrap();
                let result = child.wait_with_output().unwrap();
                assert!(
                    result.status.success(),
                    "{python_version}, {optimization_level:?}: {}",
                    String::from_utf8_lossy(&result.stderr)
                );
            }
        }
    }
}

#[test]
fn legacy_normal_finalizer_pass_keeps_outer_handler() {
    const TRACE: &str = r"
import dis, marshal, sys

events = []
def work():
    events.append('work')
def after():
    events.append('after')
def recover():
    events.append('recover')
scope = {'work': work, 'after': after, 'recover': recover}
exec(marshal.loads(sys.stdin.buffer.read()), scope)
f = scope['f']
targets = [instruction.offset for instruction in dis.get_instructions(f)
           if instruction.opname == 'NOP' and instruction.positions.lineno == 10]
if not targets:
    raise RuntimeError('missing pass marker')
target = min(targets)
generator, has_sibling = (argument == '1' for argument in sys.argv[1:])
monitor = sys.monitoring
monitor.use_tool_id(5, 'legacy-normal-finalizer')
for fault in (ValueError, StopIteration):
    for flag in (False, True):
        events.clear()
        hits = []
        def inject(code, offset):
            if offset == target:
                hits.append(offset)
                raise fault('injected')
        monitor.register_callback(5, monitor.events.INSTRUCTION, inject)
        monitor.set_local_events(5, f.__code__, monitor.events.INSTRUCTION)
        try:
            value = f(None, flag)
            if generator:
                value = next(value)
            result = ('value', value)
        except BaseException as error:
            result = ('error', type(error))
        finally:
            monitor.set_local_events(5, f.__code__, 0)
            monitor.register_callback(5, monitor.events.INSTRUCTION, None)
        expected = ('value', 1) if has_sibling else (
            'error', RuntimeError if generator and fault is StopIteration else fault)
        expected_events = ['work', 'recover'] if has_sibling else ['work']
        if hits != [target] or events != expected_events or result != expected:
            raise RuntimeError((hits, events, result, expected))
monitor.free_tool_id(5)
";
    for generator in [false, true] {
        for has_sibling in [false, true] {
            let sibling = if has_sibling { "        after()\n" } else { "" };
            let exit = if generator { "yield 1" } else { "return 1" };
            let source = format!(
                "def f(manager,flag):\n    try:\n        try:\n            work()\n        finally:\n            try:\n                if flag:\n                    pass\n            finally:\n                pass\n{sibling}    except Exception:\n        recover()\n    {exit}\n"
            );
            for optimization_level in LEVELS {
                let native = oracle(
                    source.as_bytes(),
                    "legacy_normal_finalizer.py",
                    PythonVersion::Py312,
                    optimization_level,
                );
                assert!(
                    native.status.success(),
                    "{}",
                    String::from_utf8_lossy(&native.stderr)
                );
                let actual = compile_with_options(
                    &source,
                    "legacy_normal_finalizer.py",
                    &CompileOptions {
                        python_version: PythonVersion::Py312,
                        optimization_level,
                        ..CompileOptions::default()
                    },
                )
                .unwrap()
                .marshal();
                assert_eq!(
                    actual, native.stdout,
                    "{optimization_level:?}, {generator}, {has_sibling}"
                );
                for code in [&native.stdout, &actual] {
                    let mut child = Command::new(ORACLES[0].get().unwrap())
                        .args(["-I", "-S", "-B", "-c", TRACE])
                        .args([
                            if generator { "1" } else { "0" },
                            if has_sibling { "1" } else { "0" },
                        ])
                        .stdin(Stdio::piped())
                        .stdout(Stdio::piped())
                        .stderr(Stdio::piped())
                        .spawn()
                        .unwrap();
                    child.stdin.take().unwrap().write_all(code).unwrap();
                    let result = child.wait_with_output().unwrap();
                    assert!(
                        result.status.success(),
                        "{optimization_level:?}, {generator}, {has_sibling}: {}",
                        String::from_utf8_lossy(&result.stderr)
                    );
                }
            }
        }
    }
}

#[test]
fn terminal_try_pass_preserves_implicit_return_location() {
    for source in [
        "def f(x):\n    try:\n        x = x + 1\n        pass\n    except ValueError:\n        recover()\n",
        "def f(x):\n    try:\n        del x\n        pass\n    except ValueError:\n        recover()\n",
        "def f(x):\n    try:\n        consume(x)\n        pass\n    except ValueError:\n        recover()\n",
        "def f(x):\n    try:\n        x = x + 1; pass\n    except ValueError:\n        recover()\n",
        "def f(x):\n    try:\n        del x; pass\n    except ValueError:\n        recover()\n",
        "def f(x):\n    try:\n        consume(x); pass\n    except ValueError:\n        recover()\n",
        "def f(x):\n    try:\n        x = (x +\n             1); pass\n    except ValueError:\n        recover()\n",
        "def f(x):\n    try:\n        consume(\n            x,\n        ); pass\n    except ValueError:\n        recover()\n",
        "def f(x, y):\n    try:\n        if y:\n            assert x\n        else:\n            assert y\n        z = x + 1\n        pass\n    except ValueError:\n        recover()\n",
        "def f(x, y):\n    try:\n        if y:\n            assert x\n        else:\n            assert y\n        del x\n        pass\n    except ValueError:\n        recover()\n",
    ] {
        assert_matches_cpython_versions(
            source,
            "terminal_try_pass.py",
            &LEVELS,
            &[PythonVersion::Py314, PythonVersion::Py315],
        );
    }
}

#[test]
fn terminal_try_consecutive_passes_preserve_locations() {
    for source in [
        "def f(x):\n    try:\n        x = x + 1\n        pass\n        pass\n    except ValueError:\n        recover()\n",
        "def f(x):\n    try:\n        consume(\n            x,\n        ); pass; pass\n    except ValueError:\n        recover()\n",
        "def f(x):\n    try:\n        x = x + 1\n        pass; pass\n    except ValueError:\n        recover()\n",
        "def f(x):\n    try:\n        x = x + 1\n        pass\n        pass\n        pass\n    except ValueError:\n        recover()\n",
        "def f(x):\n    try:\n        x = x + 1\n        pass; pass\n        pass\n    except ValueError:\n        recover()\n",
        "def f(x):\n    try: pass; pass\n    except ValueError: recover()\n",
        "def f(x):\n    try:\n        try:\n            consume(x)\n        finally:\n            pass\n        pass\n        pass\n    except ValueError:\n        recover()\n",
    ] {
        assert_matches_cpython_versions(
            source,
            "terminal_try_consecutive_passes.py",
            &LEVELS,
            &[
                PythonVersion::Py312,
                PythonVersion::Py313,
                PythonVersion::Py314,
                PythonVersion::Py315,
            ],
        );
    }

    for source in [
        "def f(x):\n    try:\n        consume(x); pass; pass\n    except ValueError:\n        recover()\n",
        "def f(xs, flag):\n    try:\n        z = [v for v in xs] if flag else []\n        pass\n        pass\n        pass\n    except ValueError:\n        recover()\n",
    ] {
        assert_matches_cpython_versions(
            source,
            "terminal_try_consecutive_passes_modern.py",
            &LEVELS,
            &[PythonVersion::Py314, PythonVersion::Py315],
        );
    }
}

#[test]
fn terminal_conditional_comprehension_pass_keeps_return_location() {
    for source in [
        "def f(xs, flag):\n    try:\n        z = [v for v in xs] if flag else []\n        pass\n    except ValueError:\n        recover()\n",
        "def f(xs, flag):\n    try:\n        z = {v for v in xs} if not flag else []; pass\n    except ValueError:\n        recover()\n",
        "def f(xs, flag):\n    try:\n        z = {v: v for v in xs} if check(flag) else []\n        pass\n    except ValueError:\n        recover()\n",
        "def f(xs, flag):\n    try:\n        with manager():\n            pass\n            global other\n        z = [v for v in xs if v] if flag else []\n        pass\n    except ValueError:\n        recover()\n",
        "def f(xs, flag):\n    try:\n        try:\n            pass\n        except TypeError:\n            recover_inner()\n        z = [v for v in xs if v] if flag else []\n        pass\n    except ValueError:\n        recover()\n",
        "def f(xs, flag):\n    try:\n        try:\n            work()\n        finally:\n            pass\n        z = [v for v in xs if v] if flag else []\n        pass\n    except ValueError:\n        recover()\n",
        "def f(xs, flag):\n    global z\n    try:\n        z = [(w := v) for v in xs] if flag else []\n        pass\n    except ValueError:\n        recover()\n",
        "def outer():\n    z = None\n    def f(xs, flag):\n        nonlocal z\n        try:\n            z = [v for v in xs] if flag else []\n            pass\n        except ValueError:\n            recover()\n    return f\n",
    ] {
        assert_matches_cpython_versions(
            source,
            "terminal_conditional_comprehension_pass.py",
            &LEVELS,
            &[
                PythonVersion::Py312,
                PythonVersion::Py313,
                PythonVersion::Py314,
                PythonVersion::Py315,
            ],
        );
    }
}

#[test]
fn terminal_finally_pass_after_nested_function_matches_cpython() {
    for source in [
        "def f(flag):\n    try:\n        def inner():\n            pass\n        result = flag\n        pass\n    finally:\n        cleanup()\n",
        "def f(flag):\n    try:\n        def inner():\n            return flag\n        result = flag\n        pass\n    finally:\n        cleanup(flag)\n",
        "def f(flag):\n    try:\n        def inner():\n            pass\n        result = flag; pass\n    finally:\n        cleanup()\n",
        "def f(flag):\n    try:\n        def inner():\n            'doc'\n            return flag\n        result = (\n            flag); pass\n    finally:\n        cleanup(\n            flag\n        )\n",
        "def f(flag):\n    if False:\n        missing = 1\n    try:\n        def inner():\n            pass\n        result = missing\n        pass\n    finally:\n        cleanup()\n",
        "def f(flag, xs, obj, manager):\n    try:\n        def inner():\n            try:\n                item = [v for v in xs] if flag else []\n                pass\n            except ValueError:\n                work()\n        result = flag\n        pass\n    finally:\n        cleanup()\n",
    ] {
        assert_matches_cpython_versions(
            source,
            "terminal_finally_pass.py",
            &LEVELS,
            &[PythonVersion::Py314, PythonVersion::Py315],
        );
    }
}

#[test]
fn terminal_finally_pass_is_outside_its_cleanup_handler() {
    const SOURCE: &str = "def f(flag):\n    try:\n        def inner():\n            return flag\n        result = flag\n        pass\n    finally:\n        cleanup()\n";
    const TRACE: &str = r"
import dis, json, marshal, sys
events = []
class Fault(Exception): pass
def cleanup(): events.append('cleanup')
scope = {'cleanup': cleanup}
exec(marshal.loads(sys.stdin.buffer.read()), scope)
f = scope['f']
for opcode, line, expected in [('MAKE_FUNCTION', 3, ['cleanup', 'escaped']),
                               ('LOAD_DEREF', 5, ['cleanup', 'escaped']),
                               ('NOP', 6, ['escaped'])]:
    targets = [i.offset for i in dis.get_instructions(f)
               if i.opname == opcode and i.positions.lineno == line]
    assert len(targets) == 1, (opcode, line, targets)
    hits = []
    def trace(frame, event, arg):
        if frame.f_code is f.__code__:
            frame.f_trace_opcodes = True
            if event == 'opcode' and frame.f_lasti == targets[0]:
                hits.append(frame.f_lasti)
                raise Fault()
        return trace
    events.clear()
    sys.settrace(trace)
    try:
        f(True)
    except Fault:
        events.append('escaped')
    finally:
        sys.settrace(None)
    assert len(hits) == 1, (opcode, hits)
    assert events == expected, (opcode, events, expected)
print(json.dumps('ok'))
";
    for (index, (version, _, _)) in VERSIONS.iter().copied().enumerate().skip(2) {
        for optimization_level in LEVELS {
            let native = oracle(
                SOURCE.as_bytes(),
                "terminal_finally_pass.py",
                version,
                optimization_level,
            );
            assert!(
                native.status.success(),
                "{}",
                String::from_utf8_lossy(&native.stderr)
            );
            let actual = compile_with_options(
                SOURCE,
                "terminal_finally_pass.py",
                &CompileOptions {
                    python_version: version,
                    optimization_level,
                    ..CompileOptions::default()
                },
            )
            .unwrap()
            .marshal();
            for bytes in [&native.stdout[..], &actual] {
                let mut child = Command::new(ORACLES[index].get().unwrap())
                    .args(["-I", "-S", "-c", TRACE])
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .unwrap();
                child.stdin.take().unwrap().write_all(bytes).unwrap();
                let output = child.wait_with_output().unwrap();
                assert!(
                    output.status.success(),
                    "{}",
                    String::from_utf8_lossy(&output.stderr)
                );
                assert_eq!(output.stdout, b"\"ok\"\n");
            }
        }
    }
}

static ORACLES: [OnceLock<OsString>; 4] = [const { OnceLock::new() }; 4];

#[test]
fn optimized_for_loop_backedge_locations_match_cpython_312() {
    for source in [
        "def f(xs):\n for x in xs:\n  if x:\n   assert False\n  if x:\n   assert False\n",
        "def f(xs):\n for x in xs:\n  if x:\n   assert False\n  if x:\n   assert False\n  if x:\n   assert False\n",
        "def f(xs):\n for x in xs:\n  if not x:\n   assert False\n  if not x:\n   assert False\n",
        "def f(xs, pred):\n for x in xs:\n  if pred(x):\n   assert False\n  if pred(x):\n   assert False\n",
        "def f(xs):\n for y in xs:\n  for x in xs:\n   if x:\n    assert False\n   if x:\n    assert False\n",
        "def f(xs):\n for x in xs:\n  if x:\n   assert False\n",
        "def f(xs, y):\n for x in xs:\n  if x and y:\n   assert False\n  if x and y:\n   assert False\n",
        "def f(xs, y):\n for x in xs:\n  if x or y:\n   assert False\n  if x or y:\n   assert False\n",
        "def f(xs):\n for x in xs:\n  if x:\n   assert False\n  if x:\n   assert False\n  pass\n",
        "def f(xs):\n for x in xs:\n  if x:\n   assert False\n  if x:\n   assert False\n  continue\n",
        "def f(x):\n while x:\n  if x:\n   assert False\n  if x:\n   assert False\n",
    ] {
        assert_matches_cpython_versions(
            source,
            "loop_backedge_locations.py",
            &LEVELS,
            &[PythonVersion::Py312],
        );
    }
}

#[test]
fn protected_for_loop_backedge_locations_match_cpython_312() {
    for source in [
        "def f(xs, cleanup):\n try:\n  for x in xs:\n   if x:\n    assert False\n   if x:\n    assert False\n finally:\n  cleanup()\n",
        "def f(xs, manager):\n with manager:\n  for x in xs:\n   if x:\n    assert False\n   if x:\n    assert False\n",
        "def f(xs, x):\n for item in xs:\n  if x:\n   assert False\n  if x:\n   assert False\n try:\n  z = x + 1\n  pass\n except ValueError:\n  recover()\n",
    ] {
        assert_matches_cpython_versions(
            source,
            "protected_loop_backedge_locations.py",
            &LEVELS,
            &[PythonVersion::Py312],
        );
    }
}

#[test]
fn terminating_if_before_elided_assert_preserves_branch_location() {
    for source in [
        "def f(a,b,c):\n if a:\n  if b:\n   raise ValueError()\n  assert a\n else:\n  other()\n after()\n",
        "def f(a,b,c):\n if a:\n  if not b:\n   raise ValueError()\n  assert a\n else:\n  other()\n after()\n",
        "def f(a,b,c):\n if a:\n  if b or c:\n   raise ValueError()\n  assert a\n else:\n  other()\n after()\n",
        "def f(a,b,c):\n if a:\n  if b < c:\n   return 3\n  assert a\n else:\n  other()\n after()\n",
        "def f(a,b,c):\n if a:\n  other()\n elif c:\n  if check(b):\n   return work(b)\n  assert a\n  assert (z := b)\n else:\n  other()\n after()\n",
        "def f(a,b,c):\n if a:\n  if b:\n   raise ValueError()\n  assert a\n  pass\n else:\n  other()\n after()\n",
        "def f(a,b,c):\n if a:\n  if True:\n   raise ValueError()\n  assert a\n else:\n  other()\n after()\n",
        "def f(a,b,c,cm,xs):\n if a:\n  try:\n   work()\n  finally:\n   cleanup()\n  if b:\n   raise ValueError()\n  assert a\n else:\n  other()\n after()\n",
    ] {
        assert_matches_cpython_versions(
            source,
            "sub/elided_terminating_if.py",
            &LEVELS,
            &[
                PythonVersion::Py312,
                PythonVersion::Py313,
                PythonVersion::Py314,
                PythonVersion::Py315,
            ],
        );
    }
}

#[test]
fn nonterminal_elided_match_case_preserves_unlocated_exit() {
    for source in [
        "def f(x):\n match x:\n  case str():\n   assert x\n  case int():\n   work()\n  case _:\n   other()\n after()\n",
        "def f(x):\n match x:\n  case None:\n   pass\n  case str() | bytes():\n   assert x\n   assert (w := x)\n  case int():\n   work()\n  case _:\n   other()\n after()\n",
        "def f(x):\n match x:\n  case str(v) | bytes(v):\n   assert (z := x)\n  case int():\n   work()\n  case _:\n   other()\n after()\n",
        "def f(x):\n match x:\n  case A(foo=y) | B(foo=y):\n   assert x\n  case int():\n   work()\n  case _:\n   other()\n after()\n",
        "def f(x):\n match x:\n  case (1, 2) | (3, 4):\n   assert x\n  case int():\n   work()\n  case _:\n   other()\n after()\n",
        "def f(x):\n global z\n match x:\n  case str() | bytes():\n   assert (z := x)\n  case int():\n   work()\n  case _:\n   other()\n after()\n",
        "def f(x):\n match x:\n  case str():\n   assert x\n   pass\n  case int():\n   work()\n  case _:\n   other()\n after()\n",
        "def f(x):\n match x:\n  case str():\n   assert x\n   work()\n  case int():\n   work()\n  case _:\n   other()\n after()\n",
    ] {
        assert_matches_cpython_versions(
            source,
            "sub/nonterminal_elided_match.py",
            &LEVELS,
            &[
                PythonVersion::Py312,
                PythonVersion::Py313,
                PythonVersion::Py314,
                PythonVersion::Py315,
            ],
        );
    }
}

#[test]
fn elided_inner_else_keeps_for_loop_backedge_location() {
    for source in [
        "def f(xs,a,b):\n for x in xs:\n  if a:\n   if b:\n    z = x\n   else:\n    assert b\n  else:\n   other(x)\n",
        "def f(xs,a,b,c):\n for x in xs:\n  if a:\n   other(x)\n  elif c:\n   if not b:\n    work(x)\n   else:\n    assert b\n    assert (w := c)\n  else:\n   other(x)\n",
        "def f(xs,a,b,c):\n for x in xs:\n  if a:\n   if b < c:\n    z += 1\n   else:\n    assert b\n  else:\n   other(x)\n",
        "def f(xs,a,b,c):\n for x in xs:\n  if a:\n   if b or c:\n    z = x\n   else:\n    assert b\n  else:\n   other(x)\n",
        "def f(xs,a,b,c):\n for x in xs:\n  if c:\n   if a:\n    work(x)\n   elif b:\n    z = x\n   else:\n    assert b\n  else:\n   other(x)\n",
        "def f(xs,a,b):\n global z\n for x in xs:\n  if a:\n   if b:\n    z = x\n   else:\n    assert b\n  else:\n   other(x)\n",
        "def f(xs,a,b):\n def read():\n  return z\n for x in xs:\n  if a:\n   if b:\n    z = x\n   else:\n    assert b\n  else:\n   other(x)\n",
        "def f(xs,a,b):\n for x in xs:\n  if a:\n   if b:\n    z = x\n   else:\n    assert b\n    pass\n  else:\n   other(x)\n",
        "def f(xs,a,b):\n for x in xs:\n  if a:\n   if True:\n    z = x\n   else:\n    assert b\n  else:\n   other(x)\n",
    ] {
        assert_matches_cpython_versions(
            source,
            "sub/elided_inner_else_loop.py",
            &LEVELS,
            &[
                PythonVersion::Py312,
                PythonVersion::Py313,
                PythonVersion::Py314,
                PythonVersion::Py315,
            ],
        );
    }
}

#[test]
fn elided_inner_else_in_for_else_matches_cpython() {
    for (prefix, finalbody) in [
        ("", "        finish()\n"),
        (
            "            if c:\n                break\n",
            "        finish()\n",
        ),
        (
            "            if c:\n                continue\n",
            "        finish()\n",
        ),
        (
            "",
            "        try:\n            finish()\n        finally:\n            cleanup()\n",
        ),
    ] {
        let source = format!(
            "def f(xs, a, b, c):\n    z = 0\n    for x in xs:\n        if a:\n{prefix}            if b:\n                z = x\n            else:\n                assert b\n        else:\n            work(x)\n    else:\n{finalbody}"
        );
        assert_matches_cpython_versions(
            &source,
            "elided_inner_else_for_else.py",
            &LEVELS,
            &[
                PythonVersion::Py312,
                PythonVersion::Py313,
                PythonVersion::Py314,
                PythonVersion::Py315,
            ],
        );
    }
}

#[test]
fn elided_inner_else_for_else_preserves_backedge_trace() {
    const SOURCE: &str = "def f(xs, a, b, c):\n    z = 0\n    for x in xs:\n        if a:\n            if c:\n                break\n            if b:\n                z = x\n            else:\n                assert b\n        else:\n            work(x)\n    else:\n        finish()\n";
    const TRACE: &str = r"
import dis, marshal, sys
scope = {}
events = []
def finish(): events.append('finish')
def work(x): events.append(('work', x))
scope.update(finish=finish, work=work)
exec(marshal.loads(sys.stdin.buffer.read()), scope)
f = scope['f']
targets = [i.offset for i in dis.get_instructions(f)
           if i.opname.startswith('JUMP_BACKWARD') and i.positions.lineno == 7]
if len(targets) != 1:
    raise RuntimeError(('backedge', targets))
target = targets[0]
if any(e.start <= target < e.end for e in dis.Bytecode(f).exception_entries):
    raise RuntimeError('backedge unexpectedly has an exception owner')
class Fault(Exception): pass
hits = []
def trace(frame, event, arg):
    if frame.f_code is f.__code__:
        frame.f_trace_opcodes = True
        if event == 'opcode' and frame.f_lasti == target:
            hits.append(target)
            raise Fault()
    return trace
sys.settrace(trace)
try:
    try:
        f([1, 2], True, False, False)
        raise RuntimeError('injection missed')
    except Fault:
        pass
finally:
    sys.settrace(None)
if hits != [target] or events:
    raise RuntimeError((hits, events))
f([1, 2], True, False, True)
if events:
    raise RuntimeError(('break ran loop else', events))
f([1, 2], True, False, False)
if events != ['finish']:
    raise RuntimeError(('normal loop else', events))
";
    for python_version in [PythonVersion::Py314, PythonVersion::Py315] {
        let index = VERSIONS
            .iter()
            .position(|(version, _, _)| *version == python_version)
            .unwrap();
        for optimization_level in [OptimizationLevel::One, OptimizationLevel::Two] {
            let native = oracle(
                SOURCE.as_bytes(),
                "elided_inner_else_for_else.py",
                python_version,
                optimization_level,
            );
            assert!(native.status.success());
            let actual = compile_with_options(
                SOURCE,
                "elided_inner_else_for_else.py",
                &CompileOptions {
                    python_version,
                    optimization_level,
                    ..CompileOptions::default()
                },
            )
            .unwrap()
            .marshal();
            assert_eq!(actual, native.stdout);
            for payload in [&native.stdout, &actual] {
                let mut child = Command::new(ORACLES[index].get().unwrap())
                    .args(["-I", "-S", "-c", TRACE])
                    .stdin(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .unwrap();
                child.stdin.take().unwrap().write_all(payload).unwrap();
                let result = child.wait_with_output().unwrap();
                assert!(
                    result.status.success(),
                    "{python_version:?} {optimization_level:?}: {}",
                    String::from_utf8_lossy(&result.stderr)
                );
            }
        }
    }
}

#[test]
fn elided_inner_else_keeps_protected_loop_backedge_location() {
    for source in [
        "def f(a,b,c,ys):\n try:\n  for v in ys:\n   if a:\n    if b:\n     z = v\n    else:\n     assert b\n   else:\n    consume(v)\n except ValueError:\n  recover()\n",
        "def f(a,b,c,ys):\n try:\n  for v in ys:\n   if a:\n    if b or c:\n     consume(v)\n    else:\n     assert b\n   else:\n    consume(v)\n  after = a\n except ValueError:\n  recover()\n",
        "def f(a,b,c,ys):\n try:\n  for v in ys:\n   if a:\n    if not b:\n     consume(v)\n    else:\n     assert b\n   else:\n    consume(v)\n  z = a + 1\n  pass\n  pass\n except ValueError:\n  recover()\n",
        "def f(a,b,c,ys):\n try:\n  for v in ys:\n   if a:\n    if b < c:\n     z = v\n    else:\n     assert b\n   else:\n    consume(v)\n  after = a\n except ValueError:\n  recover()\n else:\n  consume(0)\n",
        "def f(a,b,c,items):\n    try:\n        for item in items:\n            if a:\n                if b:\n                    consume(item)\n                elif c:\n                    consume(item)\n                else:\n                    assert b\n            else:\n                consume(item)\n        z = 1\n    except ValueError:\n        recover()\n",
        "def f(a,b,c,items):\n    try:\n        for item in items:\n            if a:\n                if b:\n                    consume(item)\n                elif c:\n                    assert c\n                else:\n                    assert b\n            else:\n                consume(item)\n        else:\n            finish()\n        z = 1\n    except ValueError:\n        recover()\n",
    ] {
        assert_matches_cpython_versions(
            source,
            "sub/protected_elided_inner_else_loop.py",
            &LEVELS,
            &[
                PythonVersion::Py312,
                PythonVersion::Py313,
                PythonVersion::Py314,
                PythonVersion::Py315,
            ],
        );
    }
}

#[test]
fn nested_terminal_if_before_elided_assert_matches_cpython() {
    for source in [
        "def f(a,b):\n if a:\n  if b:\n   x = 1\n  assert a\n",
        "def f(a,b):\n if a:\n  if b:\n   assert a\n  assert b\n",
        "def f(a,b,c):\n if a:\n  if b and c:\n   assert b\n  assert a\n",
        "def f(a,b):\n if a:\n  if b:\n   assert (x := b)\n   assert (y := a)\n  assert a\n",
        "def f(a,b):\n def read():\n  return x\n if a:\n  if b:\n   assert (x := b)\n  assert a\n",
        "def f(a,b):\n if a:\n  if b:\n   work()\n  assert a\n  assert b\n",
        "def f(a,b):\n if a:\n  if b:\n   work()\n  else:\n   assert b\n  assert a\n",
        "def f(a,b):\n if a:\n  if b:\n   work()\n  else:\n   assert (x := a)\n  assert b\n",
        "def f(a,b):\n def read():\n  return x\n if a:\n  if b:\n   work()\n  else:\n   assert (x := a)\n  assert b\n",
        "def f(a,b):\n global x\n if a:\n  if b:\n   work()\n  else:\n   assert (x := a)\n  assert b\n",
        "def outer():\n x=0\n def f(a,b):\n  nonlocal x\n  if a:\n   if b:\n    work()\n   else:\n    assert (x := a)\n   assert b\n return f\n",
        "def f(a,b,c):\n if a:\n  if b and c:\n   assert a\n  else:\n   assert b\n  assert c\n",
        "def f(a,b):\n global x\n if a:\n  if b:\n   x = 1\n  else:\n   assert a\n  assert b\n",
        "def f(a,b,x):\n if a:\n  if not b:\n   x += 1\n  assert a\n",
        "def f(a,b):\n global x\n if a:\n  if b:\n   x = 1\n  assert a\n",
        "def outer():\n x=0\n def f(a,b):\n  nonlocal x\n  if a:\n   if b:\n    x = 1\n   assert a\n return f\n",
        "def f(a,b,obj):\n if a:\n  if b:\n   left, *rest = obj\n  assert a\n else:\n  work()\n",
        "def f(a,b):\n if a:\n  if b:\n   x = 1\n  assert a\n  pass\n",
        "def f(a,b,cm):\n with cm:\n  if a:\n   if b:\n    x = 1\n   assert a\n",
        "def f(a,b):\n if a:\n  if b:\n   x = 1\n  assert a\n yield b\n",
        "async def f(a,b):\n if a:\n  if b:\n   x = 1\n  assert a\n await work()\n",
    ] {
        assert_matches_cpython_versions(
            source,
            "sub/nested_terminal_elided.py",
            &LEVELS,
            &[PythonVersion::Py314, PythonVersion::Py315],
        );
    }
}

#[test]
fn nested_loop_if_before_elided_assert_matches_cpython() {
    for source in [
        "def f(xs,a,b):\n for x in xs:\n  if a:\n   if b:\n    work(x)\n   assert x\n",
        "def f(xs,a,b):\n for x in xs:\n  if a:\n   if b:\n    assert b\n   assert x\n",
        "def f(xs,a,b):\n for x in xs:\n  if a:\n   if b:\n    a += 1\n   assert x\n",
        "def f(xs,a,b):\n while xs:\n  x=xs.pop()\n  if a:\n   if b:\n    work(x)\n   assert x\n",
        "def f(xs,a,b):\n for x in xs:\n  if a:\n   if b:\n    work(x)\n   assert x\n  else:\n   other(x)\n",
    ] {
        assert_matches_cpython_versions(
            source,
            "sub/nested_loop_elided.py",
            &LEVELS,
            &[PythonVersion::Py314, PythonVersion::Py315],
        );
    }
}

#[test]
fn terminal_if_before_elided_assert_matches_cpython_312() {
    for source in [
        "def f(x):\n if x:\n  with context():\n   call()\n else:\n  if x:\n   assert x\n  assert x\n",
        "def f(x, y):\n    if x:\n        with context():\n            call()\n    else:\n        if y:\n            pass\n        assert x\n        call()\n",
        "def f(x, y):\n    if x:\n        with context():\n            call()\n    else:\n        if x:\n            assert x\n        assert x\n        return 1\n",
        "def f(x, y):\n    if x:\n        with context():\n            call()\n    else:\n        if x:\n            assert x\n        assert x\n        raise ValueError\n",
        "def f(x, y):\n    if x:\n        with context():\n            call()\n    else:\n        if x:\n            assert x\n        assert x\n        try:\n            call()\n        finally:\n            clean()\n",
        "def f(x):\n if x:\n  call()\n else:\n  with context():\n   call()\n  if x:\n   assert x\n  assert x\n",
        "def f(x):\n if x:\n  with context():\n   call()\n else:\n  if x:\n   assert x\n",
        "def f(x):\n with context():\n  call()\n if x:\n  assert x\n assert x\n",
        "def f(x):\n if x:\n  call()\n else:\n  if x:\n   assert x\n  assert x\n",
        "def f(x):\n if x:\n  assert x\n assert x\n",
        "def f(x):\n if x:\n  try:\n   call()\n  except Exception:\n   pass\n else:\n  if x:\n   assert x\n  assert x\n",
        "def f(x):\n if x:\n  while x:\n   call()\n else:\n  if x:\n   assert x\n  assert x\n",
        "def f(x):\n if x:\n  for a in x:\n   call()\n else:\n  if x:\n   assert x\n  assert x\n",
        "def f(x):\n if x:\n  return 3\n else:\n  if x:\n   assert x\n  assert x\n",
        "def f(x):\n try:\n  if x:\n   with context():\n    call()\n  else:\n   if x:\n    assert x\n   assert x\n except Exception:\n  recover()\n",
        "def f(x):\n try:\n  if x:\n   with context():\n    call()\n  else:\n   if x:\n    assert x\n   assert x\n finally:\n  clean()\n",
        "def f(x):\n with context():\n  if x:\n   with context():\n    call()\n  else:\n   if x:\n    assert x\n   assert x\n",
        "def f(x):\n if x:\n  if x:\n   assert x\n  assert x\n else:\n  with context():\n   call()\n",
        "def f(x):\n if x:\n  with context():\n   call()\n else:\n  if x:\n   assert x\n  assert x\n  assert x\n",
        "async def f(x):\n if x:\n  with context():\n   call()\n else:\n  if x:\n   assert x\n  assert x\n",
        "def f(x):\n if x:\n  with context():\n   yield x\n else:\n  if x:\n   assert x\n  assert x\n",
    ] {
        assert_matches_cpython_versions(
            source,
            "terminal_elided_assert.py",
            &LEVELS,
            &[PythonVersion::Py312],
        );
    }
}

#[test]
fn terminal_if_before_elided_assert_keeps_manager_exception_owner_312() {
    const SOURCE: &str = "def f(x):\n if x:\n  with context():\n   call()\n else:\n  if x:\n   assert x\n  assert x\n";
    const TRACE: &str = r"
import dis, json, marshal, sys
events = []
class Manager:
    def __enter__(self):
        events.append('enter')
    def __exit__(self, kind, value, traceback):
        events.append('exit:' + kind.__name__)
        return False
scope = {'context': Manager, 'call': lambda: events.append('body')}
exec(marshal.loads(sys.stdin.buffer.read()), scope)
f = scope['f']
target = next(i.offset for i in dis.get_instructions(f)
              if i.opname == 'LOAD_GLOBAL' and i.argval == 'call')
fired = [False]
def trace(frame, event, arg):
    if frame.f_code is f.__code__:
        if event == 'call':
            frame.f_trace_opcodes = True
        elif event == 'opcode' and frame.f_lasti == target:
            fired[0] = True
            raise ValueError('injected')
    return trace
sys.settrace(trace)
try:
    f(False)
except AssertionError:
    pass
sys.settrace(None)
events.clear()
sys.settrace(trace)
try:
    f(True)
except ValueError:
    events.append('escaped')
finally:
    sys.settrace(None)
assert fired[0]
print(json.dumps(events))
";
    for optimization_level in LEVELS {
        let native = oracle(
            SOURCE.as_bytes(),
            "terminal_elided_assert.py",
            PythonVersion::Py312,
            optimization_level,
        );
        assert!(native.status.success());
        let actual = compile_with_options(
            SOURCE,
            "terminal_elided_assert.py",
            &CompileOptions {
                python_version: PythonVersion::Py312,
                optimization_level,
                ..CompileOptions::default()
            },
        )
        .unwrap()
        .marshal();
        let execute = |bytes: &[u8]| {
            let mut child = Command::new(ORACLES[0].get().unwrap())
                .args(["-I", "-S", "-c", TRACE])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            child.stdin.take().unwrap().write_all(bytes).unwrap();
            child.wait_with_output().unwrap()
        };
        for result in [execute(&native.stdout), execute(&actual)] {
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
            assert_eq!(
                result.stdout,
                b"[\"enter\", \"exit:ValueError\", \"escaped\"]\n"
            );
        }
    }
}

#[test]
fn loop_tail_elided_else_matches_cpython() {
    for source in [
        r"
def f(xs):
    n = None
    for count in xs:
        if n is None:
            n = count
        else:
            assert n == count
    return n
",
        r"
def f(xs, a, b):
    for x in xs:
        if a and b:
            work(x)
        else:
            assert x
",
        r"
def f(xs):
    for x in xs:
        if False:
            work(x)
        else:
            assert x
",
        r"
def f(xs):
    for x in xs:
        if x:
            work(x)
        else:
            assert x
            pass
",
        r"
def f(xs):
    for x in xs:
        if x:
            work(x)
        else:
            assert x
            work_else(x)
",
    ] {
        assert_matches_cpython(source, "elided_loop_else.py", &LEVELS);
    }
}

#[test]
fn elided_loop_tail_else_preserves_nested_and_suspended_branches() {
    for source in [
        r"
def f(xs, flag):
    for x in xs:
        if x:
            if flag:
                work(x)
        else:
            assert x
",
        r"
def f(xs, flag):
    for x in xs:
        if x:
            work(x)
        elif flag:
            if x > 1:
                work_else(x)
        else:
            assert x
",
        r"
def f(xs):
    for x in xs:
        if x:
            yield x
        else:
            assert x
",
    ] {
        assert_matches_cpython(source, "nested_elided_loop_else.py", &LEVELS);
    }
}

#[test]
fn protected_loop_tail_controls_match_cpython() {
    for source in [
        r"
def f(xs):
    try:
        for x in xs:
            if x:
                work(x)
            else:
                assert x
    except ValueError:
        cleanup()
",
        r"
def f(xs):
    for x in xs:
        try:
            if x:
                work(x)
            else:
                assert x
        except ValueError:
            cleanup()
",
        r"
def f(xs):
    try:
        for x in xs:
            if x:
                work(x)
            else:
                assert x
    finally:
        cleanup()
",
        r"
def f(xs, cm):
    with cm:
        for x in xs:
            if x:
                work(x)
            else:
                assert x
",
    ] {
        assert_matches_cpython_versions(
            source,
            "protected_loop_tail.py",
            &LEVELS,
            &[PythonVersion::Py312, PythonVersion::Py313],
        );
        // Optimized protected backedge ownership on newer versions is tracked separately.
        assert_matches_cpython_versions(
            source,
            "protected_loop_tail.py",
            &[OptimizationLevel::Zero],
            &[PythonVersion::Py314, PythonVersion::Py315],
        );
    }
}

#[test]
fn elided_if_else_preserves_conditional_join() {
    for source in [
        "def f(a):\n if a:\n  assert x\n else:\n  assert y\n work(a)\n return 42\n",
        "def f(a,b):\n if a and b:\n  assert x\n else:\n  assert y\n work(a)\n",
        "def f(a,b):\n if a or b:\n  assert x\n else:\n  assert y\n work(a)\n",
        "def f(a):\n if not __debug__:\n  assert x\n else:\n  assert y\n work(a)\n",
    ] {
        assert_matches_cpython(source, "sub/elided_if_else.py", &LEVELS);
    }
}

#[test]
fn protected_elided_if_else_preserves_conditional_join() {
    for source in [
        "def f(a):\n try:\n  if a:\n   assert a\n  else:\n   assert not a\n  work(a)\n except ValueError:\n  recover()\n",
        "def f(a):\n try:\n  if a is None:\n   assert a\n  else:\n   assert not a\n  work(a)\n except ValueError:\n  recover()\n else:\n  finish()\n",
        "def f(a, b):\n try:\n  if a:\n   assert a\n  else:\n   assert b\n  assert b\n except ValueError:\n  recover()\n",
        "def f(a, b):\n try:\n  if a or b:\n   assert a\n  else:\n   assert b\n  work(a)\n except ValueError:\n  recover()\n",
    ] {
        assert_matches_cpython_versions(
            source,
            "sub/protected_elided_if_else.py",
            &LEVELS,
            &[PythonVersion::Py314, PythonVersion::Py315],
        );
    }
}

#[test]
fn elided_outer_else_preserves_nested_conditional_join() {
    for source in [
        "def f(a,b):\n if a:\n  if b:\n   x = 1\n  else:\n   x = 2\n else:\n  assert a\n work(x)\n",
        "def f(a,b):\n if a and b:\n  if not b:\n   x = 1\n  else:\n   x = 2\n else:\n  assert a\n work(x)\n",
        "def f(a,b):\n if a:\n  if b:\n   work(1)\n  else:\n   work(2)\n else:\n  assert a\n finish()\n",
        "def f(a,b):\n if a:\n  if b:\n   x = 1\n  else:\n   x = 2\n else:\n  assert a\n  assert b\n work(x)\n",
        "def f(a,b):\n if a:\n  if b:\n   x = 1\n  else:\n   x = 2\n else:\n  assert a\n  work(a)\n finish()\n",
    ] {
        assert_matches_cpython(source, "sub/elided_nested_if.py", &LEVELS);
    }
}

#[test]
fn protected_elided_if_else_preserves_assignment_and_delete_join() {
    for source in [
        "def f(a):\n try:\n  if a:\n   assert a\n  else:\n   assert not a\n  a = a + 1\n except ValueError:\n  recover()\n",
        "def f(a):\n try:\n  if a:\n   assert a\n  else:\n   assert not a\n  del a\n except ValueError:\n  recover()\n",
    ] {
        assert_matches_cpython_versions(
            source,
            "sub/protected_elided_assignment_delete.py",
            &LEVELS,
            &[PythonVersion::Py314, PythonVersion::Py315],
        );
    }
}

#[test]
fn elided_if_else_in_coroutines_and_with_matches_cpython_315() {
    for source in [
        "async def f(flag):\n if flag is not False:\n  assert flag\n else:\n  assert not flag\n await work()\n",
        "def f(flag):\n with manager():\n  if flag == 'v1':\n   assert flag\n  else:\n   assert not flag\n",
        "def f(flag):\n with manager():\n  if flag:\n   assert flag\n  else:\n   assert not flag\n  work()\n",
        "async def f(flag):\n async with manager():\n  if flag:\n   assert flag\n  else:\n   assert not flag\n  await work()\n",
        "def f(flag):\n if flag:\n  assert flag\n else:\n  assert not flag\n yield flag\n",
        "def f(a,b):\n with manager():\n  if a and b:\n   assert a\n  else:\n   assert b\n  work()\n",
        "with manager():\n if flag:\n  assert flag\n else:\n  assert not flag\n work()\n",
        "class C:\n with manager():\n  if flag:\n   assert flag\n  else:\n   assert not flag\n  work()\n",
        "def f(flag):\n with manager():\n  if flag:\n   assert flag\n  else:\n   assert not flag\n  yield 1\n  raise StopIteration()\n",
        "async def f(flag):\n async with manager():\n  if flag:\n   assert flag\n  else:\n   assert not flag\n  yield 1\n  raise StopAsyncIteration()\n",
    ] {
        assert_matches_cpython_versions(
            source,
            "protected_elided_if_else.py",
            &LEVELS,
            &[PythonVersion::Py315],
        );
    }
}

#[test]
fn optimized_elided_join_preserves_exception_owners() {
    const SOURCE: &str = r"
def sync(flag):
    with manager('outer'):
        with manager('inner'):
            if flag:
                assert flag
            else:
                assert not flag
            return 1

def terminal(flag):
    with manager('terminal'):
        if flag:
            assert flag
        else:
            assert not flag

async def async_terminal(flag):
    async with manager('async'):
        if flag:
            assert flag
        else:
            assert not flag

def same_line(flag):
    with manager('same-line'):
        if flag: assert flag
        else: assert not flag

def multiple(flag):
    with manager('left'), manager('right'):
        if flag:
            assert flag
        else:
            assert not flag

def gen(flag):
    with manager('generator'):
        if flag:
            assert flag
        else:
            assert not flag
        yield 1
";
    const TRACE_JOIN: &str = r"
import asyncio, dis, marshal, sys

events = []
class Manager:
    def __init__(self, name):
        self.name = name
    def __enter__(self):
        events.append((self.name, 'enter'))
    def __exit__(self, typ, value, tb):
        events.append((self.name, 'exit', typ.__name__ if typ else None))
        return False
    async def __aenter__(self):
        events.append((self.name, 'enter'))
    async def __aexit__(self, typ, value, tb):
        events.append((self.name, 'exit', typ.__name__ if typ else None))
        return False

scope = {'manager': Manager}
exec(marshal.loads(sys.stdin.buffer.read()), scope)

def trace_join(fn, injected):
    instructions = list(dis.get_instructions(fn))
    offsets = [b.offset for a, b in zip(instructions, instructions[1:])
               if a.opname == 'NOT_TAKEN' and b.opname == 'NOP']
    assert len(offsets) == 1, offsets
    def trace(frame, event, arg):
        if frame.f_code is not fn.__code__:
            return None
        if event == 'call':
            frame.f_trace_opcodes = True
        elif event == 'opcode' and frame.f_lasti == offsets[0] and injected:
            events.append('hit')
            raise injected('injected')
        return trace
    return trace

for name, injected, expected in [
    ('sync', ValueError, [('outer', 'enter'), ('inner', 'enter'), 'hit',
                          ('inner', 'exit', 'ValueError'), ('outer', 'exit', 'ValueError'),
                          ('caught', 'ValueError', None)]),
    ('terminal', ValueError, [('terminal', 'enter'), 'hit',
                              ('terminal', 'exit', 'ValueError'), ('caught', 'ValueError', None)]),
    ('async_terminal', ValueError, [('async', 'enter'), 'hit',
                                    ('async', 'exit', 'ValueError'), ('caught', 'ValueError', None)]),
    ('same_line', ValueError, [('same-line', 'enter'), 'hit',
                               ('same-line', 'exit', 'ValueError'), ('caught', 'ValueError', None)]),
    ('multiple', ValueError, [('left', 'enter'), ('right', 'enter'), 'hit',
                              ('right', 'exit', 'ValueError'), ('left', 'exit', 'ValueError'),
                              ('caught', 'ValueError', None)]),
    ('gen', StopIteration, [('generator', 'enter'), 'hit',
                            ('generator', 'exit', 'StopIteration'),
                            ('caught', 'RuntimeError', 'StopIteration')]),
]:
    fn = scope[name]
    def invoke():
        result = fn(True)
        if name == 'gen':
            return next(result)
        if name == 'async_terminal':
            return asyncio.run(result)
        return result
    # Initialize opcode tracing for this code object before injecting an exception.
    sys.settrace(trace_join(fn, None))
    try:
        invoke()
    finally:
        sys.settrace(None)
    events.clear()
    sys.settrace(trace_join(fn, injected))
    try:
        invoke()
    except BaseException as error:
        events.append(('caught', type(error).__name__,
                       type(error.__cause__).__name__ if error.__cause__ else None))
    finally:
        sys.settrace(None)
    assert events == expected, (events, expected)
";
    for level in [OptimizationLevel::One, OptimizationLevel::Two] {
        let native = oracle(
            SOURCE.as_bytes(),
            "optimized_join.py",
            PythonVersion::Py315,
            level,
        );
        assert!(native.status.success());
        let actual = compile_with_options(
            SOURCE,
            "optimized_join.py",
            &CompileOptions {
                python_version: PythonVersion::Py315,
                optimization_level: level,
                ..CompileOptions::default()
            },
        )
        .unwrap()
        .marshal();
        assert_eq!(actual, native.stdout);
        let index = VERSIONS
            .iter()
            .position(|(version, _, _)| *version == PythonVersion::Py315)
            .unwrap();
        for (label, payload) in [
            ("native", native.stdout.as_slice()),
            ("serc", actual.as_slice()),
        ] {
            let mut child = Command::new(ORACLES[index].get().unwrap())
                .args(["-I", "-S", "-X", "utf8", "-c", TRACE_JOIN])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            child.stdin.take().unwrap().write_all(payload).unwrap();
            let result = child.wait_with_output().unwrap();
            assert!(
                result.status.success(),
                "{level:?}, {label}: {}",
                String::from_utf8_lossy(&result.stderr)
            );
        }
    }
}

#[test]
fn protected_comprehension_before_elided_assertion_keeps_return_locations() {
    for source in [
        "def f(a, ys):\n try:\n  if a:\n   x = [i for i in ys]\n  assert a\n except ValueError:\n  work()\n",
        "def f(a, ys):\n try:\n  x = 3\n  if a:\n   x = {i for i in ys}\n  assert a\n except ValueError:\n  work()\n",
        "def f(a, ys):\n try:\n  if a:\n   x = {i: i for i in ys}\n  assert a\n except:\n  work()\n",
        "def f(a, b, x, ys):\n try:\n  if a:\n   assert a\n  else:\n   assert b\n  x = 3\n  if b:\n   x = [i for i in ys]\n  assert a\n except ValueError:\n  work()\n",
    ] {
        assert_matches_cpython_versions(
            source,
            "sub/protected_comprehension_assert.py",
            &LEVELS,
            &[PythonVersion::Py314, PythonVersion::Py315],
        );
    }
}

const ORACLE_IDENTITY: &str = r"
import importlib.util, sys
assert sys.implementation.name == 'cpython'
assert sys.version.split()[0] == sys.argv[1]
assert importlib.util.MAGIC_NUMBER.hex() == sys.argv[2]
";
const MARSHAL_ORACLE: &str = r"
import marshal, sys
code = compile(sys.stdin.buffer.read(), sys.argv[2], 'exec', dont_inherit=True, optimize=int(sys.argv[1]))
sys.stdout.buffer.write(marshal.dumps(code))
";

fn oracle(
    source: &[u8],
    filename: &str,
    version: PythonVersion,
    level: OptimizationLevel,
) -> Output {
    let index = VERSIONS
        .iter()
        .position(|(candidate, _, _)| *candidate == version)
        .unwrap();
    let executable = ORACLES[index].get_or_init(|| {
        let (_, variable, fallback) = VERSIONS[index];
        let executable = std::env::var_os(variable).unwrap_or_else(|| fallback.into());
        let target = version.target();
        let identity = Command::new(&executable)
            .args(["-I", "-S", "-c", ORACLE_IDENTITY])
            .args([target.tag.trim_start_matches('v'), &target.magic_hex()])
            .output()
            .expect("check the pinned CPython oracle");
        assert!(
            identity.status.success(),
            "{variable} must name {} with magic {}: {}",
            target.tag,
            target.magic_hex(),
            String::from_utf8_lossy(&identity.stderr)
        );
        executable
    });
    let level = match level {
        OptimizationLevel::Zero => "0",
        OptimizationLevel::One => "1",
        OptimizationLevel::Two => "2",
    };
    let mut child = Command::new(executable)
        .args(["-I", "-S", "-X", "utf8", "-c", MARSHAL_ORACLE])
        .args([level, filename])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start the pinned CPython oracle");
    child.stdin.take().unwrap().write_all(source).unwrap();
    child.wait_with_output().unwrap()
}

fn assert_matches_cpython(source: &str, filename: &str, levels: &[OptimizationLevel]) {
    let versions = VERSIONS.map(|(version, _, _)| version);
    assert_matches_cpython_versions(source, filename, levels, &versions);
}

fn assert_matches_cpython_versions(
    source: &str,
    filename: &str,
    levels: &[OptimizationLevel],
    versions: &[PythonVersion],
) {
    for &python_version in versions {
        for &optimization_level in levels {
            let options = CompileOptions {
                python_version,
                optimization_level,
                ..CompileOptions::default()
            };
            let expected = oracle(
                source.as_bytes(),
                filename,
                python_version,
                optimization_level,
            );
            assert!(
                expected.status.success(),
                "{python_version}, {optimization_level:?}, {filename}: {}",
                String::from_utf8_lossy(&expected.stderr)
            );
            let actual = compile_with_options(source, filename, &options)
                .unwrap_or_else(|error| {
                    panic!("{python_version}, {optimization_level:?}, {filename}: {error}")
                })
                .marshal();
            let first_difference = actual
                .iter()
                .zip(&expected.stdout)
                .position(|(actual, expected)| actual != expected);
            assert!(
                actual == expected.stdout,
                "{python_version}, {optimization_level:?}, {filename}: first difference \
                 {first_difference:?}, lengths {} and {}",
                actual.len(),
                expected.stdout.len()
            );
        }
    }
}

#[test]
fn assertions_match_cpython_at_every_optimization_level() {
    assert_matches_cpython(
        r"
assert ready, describe()
def check(value):
    assert value
    assert value > 0, f'bad value: {value}'
    return value
def only_assert():
    assert side_effect(), message()
class C:
    assert ready
    def method(self):
        assert self.ready
try:
    effect()
finally:
    assert ready
try:
    assert ready
except ValueError:
    assert recovery
try:
    assert ready
except:
    assert recovery
after = 1
try:
    assert ready
except:
    assert recovery
finally:
    after = 2
",
        "optimized_assertions.py",
        &LEVELS,
    );
}

#[test]
fn removed_assertions_preserve_scope_and_generator_metadata() {
    assert_matches_cpython(
        r"
def assigned():
    assert (local := value)
    return local
def outer(captured):
    def inner():
        assert captured
    return inner
def captured_by_assert(local):
    assert lambda: local
def generator():
    assert (yield value)
def delegated():
    assert (yield from values)
async def async_generator():
    assert (yield value)
async def coroutine():
    assert await value
",
        "optimized_assertion_scopes.py",
        &LEVELS,
    );
}

#[test]
fn optimized_legacy_await_assertions_preserve_coroutine_symbols() {
    let sources = [
        "def f():\n    assert await g()\n",
        "def f():\n    assert condition, (await g())\n",
        "def f():\n    assert 1 and (await g())\n",
        "def f():\n    assert [await g() for x in xs]\n",
        "def f():\n    assert [x for x in await g()]\n",
        "def f():\n    assert [x async for x in xs]\n",
        "def f():\n    assert {x async for x in xs}\n",
        "def f():\n    assert {x: x async for x in xs}\n",
        "def f():\n    assert (x for x in await g())\n",
        "def f():\n    assert (await g() for x in xs)\n",
        "def f():\n    assert (x async for x in xs)\n",
        "def f():\n    assert [x for x in (x async for x in xs)]\n",
        "def f():\n    assert (lambda: (await g()))\n",
        "def f():\n    assert (lambda x=(await g()): x)\n",
        "def f():\n    assert [(lambda: (await g())) for x in xs]\n",
        "def f():\n    assert (lambda x=[y async for y in ys]: x)\n",
        "def f():\n    assert (yield 1), (await g())\n",
        "def f():\n    assert (yield from (await g()))\n",
        "def f():\n    assert (await (yield from g()))\n",
        "def f():\n    assert (x async for x in await g())\n",
        "def f():\n    assert [await x async for x in xs]\n",
        "def f():\n    assert (lambda x=(await g()): (yield 1))\n",
        "def f():\n    if condition:\n        assert await g()\n    return 1\n",
        "def f():\n    if False:\n        assert await g()\n    return 1\n",
        "def outer():\n    def inner():\n        assert await g()\n    return inner\n",
        "class C:\n    def method(self):\n        assert await g()\n",
        "def f():\n    assert condition\n",
        "def f():\n    assert (lambda: (await g()))\n    yield 1\n",
        "def f():\n    assert await g()\n    def inner(): yield 1\n    return 3\n",
        "def f():\n    assert await g()\n    yield 1\n    return\n",
        "def f():\n    assert await g()\n    yield from xs\n",
        "def ordinary():\n    assert condition\n    yield 1\n    return 3\ndef special():\n    assert await g()\n",
        "def ordinary():\n    assert condition\n    yield 1\n    return 3\n# async await\n",
    ];
    for version in [PythonVersion::Py312, PythonVersion::Py313] {
        for level in [OptimizationLevel::One, OptimizationLevel::Two] {
            for source in sources {
                let filename = "legacy_assert_await.py";
                let expected = oracle(source.as_bytes(), filename, version, level);
                assert!(
                    expected.status.success(),
                    "{version} {level:?}: {source}: {expected:?}"
                );
                let actual = compile_with_options(
                    source,
                    filename,
                    &CompileOptions {
                        python_version: version,
                        optimization_level: level,
                        ..CompileOptions::default()
                    },
                )
                .unwrap()
                .marshal();
                assert_eq!(actual, expected.stdout, "{version} {level:?}: {source}");
            }
        }
    }
}

#[test]
fn optimized_legacy_await_and_yield_reject_value_returns() {
    for source in [
        "def f():\n    assert await g()\n    yield 1\n    return 3\n",
        "def f():\n    assert await g()\n    return (yield 1)\n",
        "def f():\n    assert await g()\n    yield 1\n    if False:\n        return 3\n",
        "def f():\n    assert await g()\n    def inner(x=(yield 1)): pass\n    return 3\n",
    ] {
        for version in [PythonVersion::Py312, PythonVersion::Py313] {
            for level in [OptimizationLevel::One, OptimizationLevel::Two] {
                let expected = oracle(source.as_bytes(), "legacy_assert_await.py", version, level);
                assert!(!expected.status.success());
                assert!(
                    String::from_utf8_lossy(&expected.stderr)
                        .contains("'return' with value in async generator")
                );
                let error = compile_with_options(
                    source,
                    "legacy_assert_await.py",
                    &CompileOptions {
                        python_version: version,
                        optimization_level: level,
                        ..CompileOptions::default()
                    },
                )
                .unwrap_err();
                let serc::CompileError::InvalidSyntax { message, range } = error else {
                    panic!("{version} {level:?}: {error:?}");
                };
                assert_eq!(message, "`return` with value in async generator");
                assert_eq!(range.start().to_usize(), source.rfind("return ").unwrap());
                assert_eq!(range.end().to_usize(), source.trim_end().len());
            }
        }
    }
}

#[test]
fn optimized_legacy_await_return_preserves_symbol_error_precedence() {
    for (source, messages) in [
        (
            "def f():\n    assert await g()\n    yield 1\n    return 3\n    global x\n    x = 2\n    global x\n",
            None,
        ),
        (
            "def f():\n    assert await g()\n    yield 1\n    return 3\n    print(x)\n    global x\n",
            None,
        ),
        (
            "def f():\n    assert await g()\n    yield 1\n    return 3\n    nonlocal x\n",
            None,
        ),
        (
            "def f():\n    assert await g()\n    yield 1\n    return 3\n    [(yield x) for x in xs]\n",
            None,
        ),
        (
            "def f():\n    assert await g()\n    yield 1\n    return 3\n    [x for x in (x := xs)]\n",
            None,
        ),
        (
            "def f():\n    assert await g()\n    yield 1\n    return 3\n    [(x := 2) for x in xs]\n",
            Some((
                "assignment expression cannot rebind comprehension iteration variable 'x'",
                "assignment expression cannot rebind comprehension variable",
            )),
        ),
        (
            "def f():\n    assert await g()\n    yield 1\n    return 3\n    x = 2\n    global x\n    [(yield y) for y in ys]\n",
            None,
        ),
        (
            "def f():\n    assert await g()\n    yield 1\n    return 3\n    [(yield y) for y in ys]\n    x = 2\n    global x\n",
            None,
        ),
    ] {
        for version in [PythonVersion::Py312, PythonVersion::Py313] {
            for level in [OptimizationLevel::One, OptimizationLevel::Two] {
                let expected = oracle(source.as_bytes(), "legacy_assert_await.py", version, level);
                let stderr = String::from_utf8_lossy(&expected.stderr);
                let expected_message = stderr
                    .lines()
                    .find_map(|line| line.strip_prefix("SyntaxError: "))
                    .unwrap_or_else(|| panic!("the pinned oracle failed unexpectedly: {stderr}"));
                let serc_message = if let Some((native, serc)) = messages {
                    // Preserve Serc's existing wording while checking both diagnostics.
                    assert_eq!(expected_message, native);
                    serc
                } else {
                    expected_message
                };
                let error = compile_with_options(
                    source,
                    "legacy_assert_await.py",
                    &CompileOptions {
                        python_version: version,
                        optimization_level: level,
                        ..CompileOptions::default()
                    },
                )
                .unwrap_err();
                assert!(
                    error.to_string().ends_with(serc_message),
                    "{version} {level:?}: {source}: expected {serc_message:?}, got {error}"
                );
            }
        }
    }
}

#[test]
fn debug_constants_match_cpython_through_nested_expressions_and_scopes() {
    assert_matches_cpython(
        r"
debug = __debug__
constants = (__debug__, not __debug__, (__debug__,))
choice = 'debug' if __debug__ else 'optimized'
if __debug__:
    branch = 1
else:
    branch = 2
def function(first):
    while __debug__:
        break
    if not __debug__:
        first += 1
    return __debug__
def conjunction(value):
    return __debug__ and value
def disjunction(value):
    return __debug__ or value
class C:
    debug = __debug__
    def method(self, default=__debug__):
        return lambda: (__debug__, default)
values = [__debug__ for item in items if __debug__ or item]
generator = (__debug__ for item in items if not __debug__)
",
        "optimized_debug.py",
        &LEVELS,
    );
}

#[test]
fn debug_annotations_preserve_postponed_source_spelling() {
    let source =
        "value: __debug__ = 1\ndef function(arg: __debug__) -> tuple[__debug__]:\n    return arg\n";
    assert_matches_cpython(source, "optimized_annotations.py", &LEVELS);
    assert_matches_cpython(
        "assert condition\nx: int = 1\n",
        "optimized_assertion_annotation.py",
        &LEVELS,
    );
    assert_matches_cpython(
        &format!("from __future__ import annotations\n{source}"),
        "optimized_future_annotations.py",
        &LEVELS,
    );
}

#[test]
fn removed_assertion_comprehensions_do_not_allocate_temporary_locals() {
    assert_matches_cpython(
        r"
assert [item for item in values]
class C:
    assert [item for item in values]
def check(values):
    assert [item for item in values]
    after = 1
    return after
async def async_check(values):
    assert [item async for item in values]
def lexical_before(values):
    item = 1
    assert [item for item in values]
    return item
def lexical_after(values):
    assert [item for item in values]
    after = 1
    item = 2
    return (after, item)
def captured_comprehension(values):
    assert [lambda: item for item in values]
def asserted_local_reference():
    assert later
    first = 1
    later = 2
    return (first, later)
def unused_assertion_binding():
    assert (unused := 1)
    used = 2
    return used
",
        "optimized_comprehension_locals.py",
        &LEVELS,
    );
}

#[test]
fn removed_assertion_comprehensions_preserve_module_and_class_cells() {
    assert_matches_cpython(
        r"
assert [lambda: item for item in values]
class C:
    assert [lambda: item for item in values]
",
        "optimized_comprehension_cells.py",
        &LEVELS,
    );
    assert_matches_cpython(
        r"
assert [lambda: _ for _ in values]
x: int = 1
class C:
    assert [lambda: _ for _ in values]
    def method(self):
        return __class__
",
        "optimized_comprehension_cell_order.py",
        &LEVELS,
    );
}

#[test]
fn comprehension_temporaries_follow_surviving_execution_order() {
    assert_matches_cpython(
        r"
assert [x for x in a]
[y for y in b]
[x for x in c]
class C:
    assert [x for x in a]
    [y for y in b]
    [x for x in c]
",
        "optimized_comprehension_order.py",
        &LEVELS,
    );
}

#[test]
fn stripped_docstrings_match_cpython_flags_constants_and_locations() {
    for (source, filename) in [
        ("\"module docstring\"\n", "optimized_module_docstring.py"),
        (
            "\"doc\"\n\"\\ud800\"\ndef f():\n    \"doc\"\n    \"\"\n    x=1\n",
            "optimized_surrogate_after_docstring.py",
        ),
        ("\"doc\"\nx: int = 1\n", "optimized_docstring_annotation.py"),
        (
            "\"doc\"\nfrom __future__ import annotations\nx: int = 1\n",
            "optimized_docstring_future_annotation.py",
        ),
        (
            r#""module docstring"
"a later string expression"
def function():
    "function docstring"
    "another string expression"
    return "value"
def empty():
    "only a docstring"
async def coroutine():
    "coroutine docstring"
    return 1
def generator():
    "generator docstring"
    yield 1
class C:
    "class docstring"
    def method(self):
        "method docstring"
        return __class__
    class Empty:
        "only a nested class docstring"
"#,
            "optimized_docstrings.py",
        ),
        (
            r#"def outer():
    "retained value"
    def inner():
        "retained value"
        return "retained value"
    return inner
"#,
            "optimized_docstring_identity.py",
        ),
    ] {
        assert_matches_cpython(source, filename, &LEVELS);
    }
}

#[test]
fn removed_assertions_do_not_create_docstrings() {
    assert_matches_cpython(
        r#"assert True
"not a module docstring"
def function():
    assert True
    "not a function docstring"
class C:
    assert True
    "not a class docstring"
"#,
        "optimized_non_docstrings.py",
        &LEVELS,
    );
}

#[test]
fn optimized_assertion_syntax_matches_the_target_version() {
    for source in [
        "assert (await value)\n",
        "def f():\n    assert (await g())\n",
        "assert (__debug__ := False)\n",
        "assert func(__debug__=1)\n",
        "assert func(x=1, x=2)\n",
        "assert [0 for x.__debug__ in []]\n",
        "assert (yield, 1)\n",
        "def f():\n    assert (value := 1)\n    global value\n",
    ] {
        for (python_version, _, _) in VERSIONS {
            for optimization_level in LEVELS {
                let expected = oracle(
                    source.as_bytes(),
                    "invalid_optimized_assert.py",
                    python_version,
                    optimization_level,
                );
                let options = CompileOptions {
                    python_version,
                    optimization_level,
                    ..CompileOptions::default()
                };
                let actual = compile_with_options(source, "invalid_optimized_assert.py", &options);
                if expected.status.success() {
                    assert_eq!(
                        actual.unwrap().marshal(),
                        expected.stdout,
                        "{python_version}, {optimization_level:?}, {source:?}"
                    );
                } else {
                    let stderr = String::from_utf8_lossy(&expected.stderr);
                    let expected_message = stderr
                        .lines()
                        .find_map(|line| line.strip_prefix("SyntaxError: "))
                        .unwrap_or_else(|| {
                            panic!("the pinned oracle failed unexpectedly: {stderr}")
                        });
                    let error = actual.unwrap_err();
                    assert!(
                        error.to_string().ends_with(expected_message),
                        "{python_version}, {optimization_level:?}, {source:?}: \
                         expected {expected_message:?}, got {error}"
                    );
                }
            }
        }
    }
}

#[test]
fn optimized_assertions_skip_codegen_only_syntax_errors() {
    assert_matches_cpython(
        "assert (yield 1)\n",
        "optimized_module_yield.py",
        &[OptimizationLevel::One, OptimizationLevel::Two],
    );
}

#[test]
fn source_decoding_preserves_optimization_options() {
    let source = b"# coding: latin-1\r\n\"caf\xe9\"\r\nassert __debug__\r\nvalue = __debug__\r\n";
    let filename = "optimized_encoding.py";
    for (python_version, _, _) in VERSIONS {
        for optimization_level in LEVELS {
            let options = CompileOptions {
                python_version,
                optimization_level,
                ..CompileOptions::default()
            };
            let expected = oracle(source, filename, python_version, optimization_level);
            assert!(
                expected.status.success(),
                "{}",
                String::from_utf8_lossy(&expected.stderr)
            );
            for actual in [
                compile_bytes_with_options(source, filename, &options).unwrap(),
                compile_bytes_with_path_and_options(source, Path::new(filename), &options).unwrap(),
            ] {
                assert_eq!(
                    actual.marshal(),
                    expected.stdout,
                    "{python_version}, {optimization_level:?}"
                );
            }
        }
    }
}

#[test]
fn default_options_preserve_unoptimized_behavior() {
    let source = "\"module docstring\"\nassert condition\nvalue = __debug__\n";
    let default = CompileOptions::default();
    assert_eq!(default.optimization_level, OptimizationLevel::Zero);
    assert_eq!(
        compile(source, "default_optimization.py")
            .unwrap()
            .marshal(),
        compile_with_options(source, "default_optimization.py", &default)
            .unwrap()
            .marshal()
    );
}

#[test]
fn optimized_deep_expression_traversal_and_cleanup() {
    const CHILD_ENV: &str = "SERC_TEST_OPTIMIZED_DEEP_EXPRESSIONS";
    if std::env::var_os(CHILD_ENV).is_some() {
        let expression = std::iter::repeat_n("value", 2_048)
            .collect::<Vec<_>>()
            .join(" + ");
        let source = format!(
            "\"module docstring\"\nvalue = 1\nassert {expression}\nresult = __debug__ + {expression}\n"
        );
        for optimization_level in [OptimizationLevel::One, OptimizationLevel::Two] {
            let options = CompileOptions {
                optimization_level,
                ..CompileOptions::default()
            };
            let module = compile_with_options(&source, "deep_optimized.py", &options).unwrap();
            assert!(!module.marshal().is_empty());
        }
        return;
    }

    // A stack-overflow regression must fail this test without aborting the entire test binary.
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "optimized_deep_expression_traversal_and_cleanup",
            "--nocapture",
        ])
        .env(CHILD_ENV, "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "optimized expression subprocess failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn optimized_conditional_joins_match_cpython() {
    for source in [
        "def f(xs, a, b, c):\n    for x in xs:\n        if x == a:\n            assert x\n        elif x == b or x == c:\n            assert x\n",
        "def f(xs, a, b):\n    for x in xs:\n        if x == a or x == b:\n            assert x\n",
        "def f(x, a, b, c):\n    if x == a:\n        assert x\n    elif x == b or x == c:\n        assert x\n",
        "def f(xs, a, b, c):\n    for x in xs:\n        if x == a:\n            work(x)\n        elif x == b and x == c:\n            work(x)\n        else:\n            assert x\n    else:\n        fallback()\n",
        "def f(xs, a, b, c, gate):\n    if gate:\n        for x in xs:\n            if x == a:\n                work(x)\n            elif x == b and x == c:\n                work(x)\n            else:\n                assert x\n    else:\n        fallback()\n",
        "def f(xs, a, b, c):\n    for x in xs:\n        if x == a:\n            pass\n        elif x == b or x == c:\n            pass\n",
        "def f(xs, a, b, c):\n    for x in xs:\n        if x == a:\n            assert x\n        elif x == b or x == c:\n            assert x\n        work(x)\n",
        "events = []\na = True\nb = False\nc = True\nif b:\n    assert a\nelif a or c:\n    assert a\n",
        "events = []\na = True\nb = False\nc = True\nif b:\n    events.append('first')\nelif a or c:\n    assert a\n",
    ] {
        assert_matches_cpython(source, "case.py", &LEVELS);
    }
}

#[test]
fn optimized_conditional_join_in_exception_group_matches_cpython() {
    for source in [
        "def f(xs, a, b):\n    try:\n        work()\n    except* ValueError:\n        for x in xs:\n            if a or b:\n                assert x\n            else:\n                assert x\n",
        "def f(xs, a, b):\n    try:\n        work()\n    except* ValueError as err:\n        for x in xs:\n            if a or b:\n                assert x\n            else:\n                assert x\n",
    ] {
        assert_matches_cpython(source, "case.py", &LEVELS[1..]);
    }
}

#[test]
fn optimized_nested_else_join_locations_match_cpython() {
    for source in [
        "def f(x, obj, flag, other, items):\n    if flag:\n        if x == 0:\n            obj.channel = 0\n        else:\n            assert False, \"unknown\"\n    elif other:\n        obj.x = 42\n    obj.result = 3\n",
        "def f(x, y, z, obj, flag, items, manager):\n    try:\n        obj.before = 0\n        if flag:\n            if not (x < y < z):\n                obj.channel = 1\n            else:\n                pass\n        elif y:\n            obj.other = 2\n        obj.after = 3\n    finally:\n        obj.cleaned = True\n",
        "def f(x, y, z, obj, flag, items, manager):\n    obj.before = 0\n    if flag:\n        if x < y < z and flag:\n            try:\n                obj.channel = 1\n            finally:\n                obj.done = True\n        else:\n            assert y\n    elif y:\n        obj.other = 2\n    obj.after = 3\n",
        "def f(x, y, z, obj, flag, items, manager):\n    obj.before = 0\n    if flag:\n        if probe(x) and (y or z):\n            obj.channel = 1\n        else:\n            assert y\n    elif y:\n        obj.other = 2\n    obj.after = 3\n",
    ] {
        assert_matches_cpython(source, "case.py", &LEVELS);
    }
}

#[test]
fn optimized_nested_else_with_declarations_matches_cpython() {
    for source in [
        "def f(flag, x, y, obj):\n    if flag:\n        if x:\n            obj.value = 1\n        elif y:\n            obj.value = 2\n        else:\n            global g\n            assert x\n    elif y:\n        obj.value = 3\n    obj.done = True\n",
        "def f(flag, x, y, obj):\n    if flag:\n        if x:\n            obj.value = 1\n        else:\n            assert x\n            global g\n    elif y:\n        obj.value = 2\n    obj.done = True\n",
        "def outer():\n    captured = None\n    def f(flag, x, y, obj):\n        if flag:\n            if x:\n                obj.value = 1\n            else:\n                nonlocal captured\n                assert x\n        elif y:\n            obj.value = 2\n        obj.done = True\n    return f\n",
        "def f(flag, x, y, obj):\n    if flag:\n        if x:\n            obj.value = 1\n        else:\n            global g\n            assert x\n            pass\n    elif y:\n        obj.value = 2\n    obj.done = True\n",
    ] {
        assert_matches_cpython(source, "case.py", &LEVELS);
    }
}

#[test]
fn optimized_empty_manager_bodies_match_cpython() {
    for (header, manager) in [
        ("def f():", "with"),
        ("async def f():", "with"),
        ("async def f():", "async with"),
    ] {
        for body in [
            "        assert predicate()\n",
            "        assert predicate()\n        assert other()\n",
            "        pass\n",
            "        global marker\n        pass\n",
            "        global marker\n        assert predicate()\n",
        ] {
            for continuation in ["", "    after = work()\n"] {
                let source = format!(
                    "{header}\n    before = 1\n    {manager} manager(\n        1,\n        option=True,\n    ) as context:\n{body}{continuation}"
                );
                assert_matches_cpython(&source, "case.py", &LEVELS);
            }
        }
    }

    assert_matches_cpython(
        "def outer():\n    captured = 1\n    def f(m):\n        with m as token:\n            nonlocal captured\n            assert predicate()\n        return captured, token\n    return f\n",
        "case.py",
        &LEVELS,
    );
}

#[test]
fn protected_manager_multiple_passes_match_cpython() {
    let versions = [PythonVersion::Py314, PythonVersion::Py315];
    for source in [
        r"def f(manager, other):
    try:
        with manager as value:
            pass
            pass
            global marker
    finally:
        cleanup()
    return value
",
        r"def f(manager, other):
    try:
        with manager as value:
            pass
            global marker
            pass
    finally:
        cleanup()
    return value
",
        r"def f(manager, other):
    try:
        with manager as value: pass; pass; global marker
    finally:
        cleanup()
    return value
",
        r"def f(manager, other):
    with other:
        with manager as value: pass; pass; global marker
    return value
",
        r"def f(manager, other):
    try:
        with manager as value:
            pass
            global marker
            pass
            global spare_marker
    finally:
        cleanup()
    return value
",
        r"def f(manager, other):
    try:
        with manager as value:
            pass
            pass
            pass
    finally:
        cleanup()
    return value
",
        r"def f(manager, other):
    try:
        with manager:
            pass
            pass
            global marker
    finally:
        cleanup()
    return 42
",
        r"def f(manager, other):
    try:
        with manager: pass; pass; global marker
    finally:
        cleanup()
    return 42
",
    ] {
        assert_matches_cpython_versions(source, "case.py", &LEVELS, &versions);
    }
    for source in [
        r"def f(manager, other):
    try:
        with manager as value:
            pass
            assert predicate()
            pass
            global marker
    finally:
        cleanup()
    return value
",
        r"def f(manager, other):
    try:
        with manager:
            pass
            assert predicate()
            pass
            global marker
    finally:
        cleanup()
    return 42
",
    ] {
        assert_matches_cpython_versions(source, "case.py", &LEVELS[1..], &versions);
    }
}

#[test]
fn protected_manager_multiple_passes_with_nonlocal_match_cpython() {
    let versions = [PythonVersion::Py314, PythonVersion::Py315];
    for source in [
        r"def outer(manager, other):
    marker = 0
    def inner():
        try:
            with manager:
                pass
                pass
                nonlocal marker
        finally:
            cleanup()
    return inner
",
        r"def outer(manager, other):
    marker = 0
    def inner():
        try:
            with manager as value:
                pass
                nonlocal marker
                pass
        finally:
            cleanup()
    return inner
",
        r"def outer(manager, other):
    marker = 0
    def inner():
        try:
            with manager: pass; pass; nonlocal marker
        finally:
            cleanup()
    return inner
",
    ] {
        assert_matches_cpython_versions(source, "case.py", &LEVELS, &versions);
    }
}

#[test]
fn manager_pass_before_declarations_keeps_tail_unprotected() {
    let versions = [PythonVersion::Py314, PythonVersion::Py315];
    for source in [
        r"def f(manager, other):
    with manager as value:
        global marker
        pass
        global spare_marker
    return value
",
        r"def f(manager, other):
    with manager as value:
        pass
        pass
        global marker
    return value
",
        r"def f(manager, other):
    with other:
        with manager as value:
            pass
            pass
            global marker
    return value
",
        r"def outer():
    captured = 1
    def f(manager, other):
        with manager as value:
            pass
            pass
            nonlocal captured
        return value
    return f
",
        r"def f(manager, other):
    with manager:
        global marker
        pass
        global spare_marker
    return 42
",
        r"def f(manager, other):
    with manager:
        pass
        pass
        global marker
    return 42
",
    ] {
        assert_matches_cpython_versions(source, "case.py", &LEVELS, &versions);
    }
}

#[test]
fn legacy_manager_declaration_only_matches_cpython() {
    let versions = [PythonVersion::Py312, PythonVersion::Py313];
    for source in [
        r"def f(manager):
    with manager as value:
        global marker
    return value
",
        r"def f(manager):
    with manager:
        global marker
        global spare
    return 7
",
        r"def outer(manager):
    marker = 1
    def inner():
        with manager as value:
            nonlocal marker
        return value
    return inner
",
        r"async def f(manager):
    async with manager as value:
        global marker
    return value
",
        r"def f(manager):
    try:
        with manager:
            global marker
    finally:
        cleanup()
",
        r"def f(manager):
    with manager as value:
        assert predicate()
        global marker
    return value
",
        r"def f(manager):
    with manager as value:
        global marker
        assert predicate()
    return value
",
        r"def f(manager):
    try:
        with manager as value:
            global marker
            pass
    finally:
        cleanup()
    return value
",
        r"def f(manager):
    with manager as value:
        pass
    return value
",
        r"def f(manager):
    with manager as value: global marker
    return value
",
    ] {
        assert_matches_cpython_versions(source, "case.py", &LEVELS, &versions);
    }
}

#[test]
fn legacy_protected_manager_pass_before_assertion_matches_cpython() {
    let versions = [PythonVersion::Py312, PythonVersion::Py313];
    for source in [
        r"def outer():
    outer_marker = 0
    def f(outer, inner, obj, predicate, cleanup, flag):
        try:
            with inner:
                pass
                assert predicate()
        finally:
            cleanup()
        return
    return f
",
        r"def outer():
    outer_marker = 0
    def f(outer, inner, obj, predicate, cleanup, flag):
        try:
            with inner as value:
                pass
                global spare
                assert predicate()
        finally:
            cleanup()
        return
    return f
",
        r"def outer():
    outer_marker = 0
    def f(outer, inner, obj, predicate, cleanup, flag):
        try:
            with inner:
                pass
                pass
                nonlocal outer_marker
                assert True
        finally:
            cleanup()
        return
    return f
",
        r"def outer():
    outer_marker = 0
    def f(outer, inner, obj, predicate, cleanup, flag):
        try:
            with inner as obj.value:
                pass
                assert False
        except Exception:
            cleanup()
        return
    return f
",
        r"def outer():
    outer_marker = 0
    def f(outer, inner, obj, predicate, cleanup, flag):
        try:
            with outer:
                with inner:
                    assert predicate()
                    pass
                    assert predicate()
        finally:
            cleanup()
        return
    return f
",
        r"def outer():
    outer_marker = 0
    async def f(outer, inner, obj, predicate, cleanup, flag):
        try:
            async with outer:
                with inner:
                    pass
                    assert predicate()
        finally:
            cleanup()
        return
    return f
",
        r"def outer():
    outer_marker = 0
    def f(outer, inner, obj, predicate, cleanup, flag):
        try:
            with inner as value: pass; assert predicate()
        finally:
            cleanup()
        return
    return f
",
        r"def outer():
    outer_marker = 0
    def f(outer, inner, obj, predicate, cleanup, flag):
        with inner:
            pass
            assert predicate()
        return
    return f
",
    ] {
        assert_matches_cpython_versions(source, "case.py", &LEVELS, &versions);
    }
}

#[test]
fn legacy_protected_manager_pass_before_declaration_matches_cpython() {
    let versions = [PythonVersion::Py312, PythonVersion::Py313];
    for source in [
        r"def outer():
    outer_marker = 0
    def f(manager, outer, obj, predicate, cleanup):
        try:
            with manager as value:
                pass
                global marker
        finally:
            cleanup()
        return
    return f
",
        r"def outer():
    outer_marker = 0
    def f(manager, outer, obj, predicate, cleanup):
        try:
            with manager:
                pass
                nonlocal outer_marker
        finally:
            cleanup()
        return
    return f
",
        r"def outer():
    outer_marker = 0
    def f(manager, outer, obj, predicate, cleanup):
        try:
            with outer:
                with manager:
                    global spare
                    pass
                    global marker
        except Exception:
            cleanup()
        return
    return f
",
        r"def outer():
    outer_marker = 0
    async def f(manager, outer, obj, predicate, cleanup):
        try:
            async with outer:
                with manager as value:
                    pass
                    global marker
        finally:
            cleanup()
        return
    return f
",
        r"def outer():
    outer_marker = 0
    def f(manager, outer, obj, predicate, cleanup):
        with manager as value:
            pass
            assert predicate()
        return
    return f
",
        r"def outer():
    outer_marker = 0
    def f(manager, outer, obj, predicate, cleanup):
        try:
            with manager: pass; assert predicate()
        finally:
            cleanup()
        return
    return f
",
        r"def outer():
    outer_marker = 0
    async def f(manager, outer, obj, predicate, cleanup):
        try:
            async with manager as value:
                pass
                global marker
        finally:
            cleanup()
        return
    return f
",
        r"def outer():
    outer_marker = 0
    def f(manager, outer, obj, predicate, cleanup):
        try:
            with manager:
                pass
                pass
                global marker
        except Exception:
            cleanup()
        return
    return f
",
    ] {
        assert_matches_cpython_versions(source, "case.py", &LEVELS, &versions);
    }
    assert_matches_cpython_versions(
        r"def outer():
    outer_marker = 0
    def f(manager, outer, obj, predicate, cleanup):
        try:
            with manager:
                pass
                assert predicate()
        finally:
            cleanup()
        return
    return f
",
        "case.py",
        &LEVELS[1..],
        &versions,
    );
}

#[test]
fn legacy_protected_manager_multiple_passes_match_cpython() {
    let versions = [PythonVersion::Py312, PythonVersion::Py313];
    for source in [
        r"def f(manager, other):
    try:
        with manager as value:
            pass
            pass
            global marker
    finally:
        cleanup()
    return value
",
        r"def f(manager, other):
    try:
        with manager as value:
            pass
            global marker
            pass
    finally:
        cleanup()
    return value
",
        r"def f(manager, other):
    try:
        with manager as value:
            pass
            pass
    finally:
        cleanup()
    return value
",
        r"def f(manager, other):
    try:
        with manager as value:
            pass
            global marker
            pass
            global spare_marker
    finally:
        cleanup()
    return value
",
        r"def f(manager, other):
    try:
        with manager:
            pass
            pass
            global marker
    finally:
        cleanup()
    return 42
",
        r"def outer(manager, other):
    marker = 0
    def inner():
        try:
            with manager as value:
                pass
                pass
                nonlocal marker
        finally:
            cleanup()
    return inner
",
    ] {
        assert_matches_cpython_versions(source, "case.py", &LEVELS, &versions);
    }
}

#[test]
fn modern_manager_pass_before_declarations_matches_cpython() {
    let versions = [PythonVersion::Py314, PythonVersion::Py315];
    for source in [
        "def f(manager):\n    try:\n        with manager as value:\n            pass\n            global marker\n    finally:\n        cleanup()\n",
        "def f(manager):\n    try:\n        with manager as value:\n            global marker\n            pass\n            global spare_marker\n    finally:\n        cleanup()\n",
    ] {
        assert_matches_cpython_versions(source, "case.py", &LEVELS, &versions);
    }
    for source in [
        "def f(manager):\n    try:\n        with manager as value:\n            pass\n            assert predicate()\n            global marker\n    finally:\n        cleanup()\n",
        "def f(manager):\n    try:\n        with manager as value:\n            assert predicate()\n            pass\n            global marker\n    finally:\n        cleanup()\n",
    ] {
        assert_matches_cpython_versions(source, "case.py", &LEVELS[1..], &versions);
    }
}

#[test]
fn nested_continue_with_elided_assertion_matches_cpython() {
    for source in [
        "def f(xs, a, cm):\n    for x in xs:\n        if a:\n            if x:\n                continue\n            global g\n            assert x\n",
        "def outer():\n    g = None\n    def f(xs, a, cm):\n        for x in xs:\n            if a:\n                if x:\n                    continue\n                nonlocal g\n                assert x\n    return f\n",
        "def f(xs, a, b):\n    for x in xs:\n        if a:\n            if b:\n                continue\n            assert x\n",
        "def f(a, b):\n    while a:\n        if a:\n            if b:\n                continue\n            assert False\n",
        "def f(xs, a, b):\n    for x in xs:\n        if a:\n            if b:\n                continue\n            assert x\n    else:\n        cleanup()\n",
        "def f(xs, a, b):\n    for x in xs:\n        if a:\n            if b:\n                continue\n            pass\n",
        "def f(xs, a, b):\n    for x in xs:\n        if a:\n            if b:\n                continue\n            assert x\n            cleanup()\n",
        "def f(xs, a, b, cm):\n    with cm:\n        for x in xs:\n            if a:\n                if b:\n                    continue\n                assert x\n",
    ] {
        assert_matches_cpython(source, "case.py", &LEVELS);
    }
}

#[test]
fn elided_assertion_with_nested_finally_matches_cpython() {
    for source in [
        "def f(flag):\n    try:\n        operation()\n        assert flag\n    except ValueError:\n        raise\n    finally:\n        try:\n            cleanup()\n        except TypeError:\n            pass\n",
        "def f(flag):\n    try:\n        operation()\n    except ValueError:\n        raise\n    finally:\n        try:\n            cleanup()\n        except TypeError:\n            pass\n",
        "def f(flag):\n    try:\n        operation()\n        global declared\n        assert flag\n    except ValueError:\n        raise\n    finally:\n        try:\n            cleanup()\n        except TypeError:\n            pass\n",
        "def outer(flag):\n    declared = None\n    def f():\n        try:\n            operation()\n            nonlocal declared\n            assert flag\n        except ValueError:\n            raise\n        finally:\n            try:\n                cleanup()\n            except TypeError:\n                pass\n    return f\n",
        "def f():\n    try:\n        operation()\n        global g\n    except ValueError:\n        raise\n    finally:\n        try:\n            cleanup()\n        except TypeError:\n            pass\n",
        "def outer():\n    x = None\n    def f():\n        try:\n            operation()\n            nonlocal x\n        except ValueError:\n            raise\n        finally:\n            try:\n                cleanup()\n            except TypeError:\n                pass\n    return f\n",
    ] {
        assert_matches_cpython(source, "case.py", &LEVELS);
    }
}

#[test]
fn optimized_empty_try_loop_keeps_enclosing_exception_owner() {
    const TRACE_BACKEDGE: &str = r"
import dis, json, marshal, sys

events = []
def recover():
    events.append('recover')
def cleanup():
    events.append('cleanup')
def log():
    events.append('log')
scope = dict(recover=recover, cleanup=cleanup, log=log)
exec(marshal.loads(sys.stdin.buffer.read()), scope)
f = scope['f']
backedge = next(i.offset for i in dis.get_instructions(f) if i.opname == 'JUMP_BACKWARD')
fired = [False]
def trace(frame, event, arg):
    if frame.f_code is f.__code__:
        if event == 'call':
            frame.f_trace_opcodes = True
        elif event == 'opcode' and frame.f_lasti == backedge:
            fired[0] = True
            raise ValueError('injected')
    return trace
sys.settrace(trace)
# Prime opcode tracing, then reset it before the execution under test.
f([])
sys.settrace(None)
events.clear()
sys.settrace(trace)
try:
    f([1])
except ValueError:
    events.append('escaped')
finally:
    sys.settrace(None)
assert fired[0]
print(json.dumps(events))
";
    assert_matches_cpython(
        "async def f(xs):\n    for x in xs:\n        try:\n            assert x\n        except:\n            log()\n",
        "case.py",
        &LEVELS,
    );
    let cases = [
        (
            "def f(xs):\n    try:\n        for x in xs:\n            try:\n                assert x\n            except:\n                log()\n    except Exception:\n        recover()\n",
            &b"[\"recover\"]\n"[..],
        ),
        (
            "def f(xs):\n    try:\n        for x in xs:\n            try:\n                assert x\n            except:\n                log()\n    finally:\n        cleanup()\n",
            &b"[\"cleanup\", \"escaped\"]\n"[..],
        ),
    ];
    for (source, expected_events) in cases {
        for (index, (python_version, _, _)) in VERSIONS.iter().copied().enumerate() {
            for optimization_level in LEVELS {
                let native = oracle(
                    source.as_bytes(),
                    "case.py",
                    python_version,
                    optimization_level,
                );
                assert!(
                    native.status.success(),
                    "{}",
                    String::from_utf8_lossy(&native.stderr)
                );
                let options = CompileOptions {
                    python_version,
                    optimization_level,
                    ..CompileOptions::default()
                };
                let actual = compile_with_options(source, "case.py", &options)
                    .unwrap()
                    .marshal();
                let execute = |bytes: &[u8]| {
                    let mut child = Command::new(ORACLES[index].get().unwrap())
                        .args(["-I", "-S", "-c", TRACE_BACKEDGE])
                        .stdin(Stdio::piped())
                        .stdout(Stdio::piped())
                        .stderr(Stdio::piped())
                        .spawn()
                        .unwrap();
                    child.stdin.take().unwrap().write_all(bytes).unwrap();
                    child.wait_with_output().unwrap()
                };
                let expected_run = execute(&native.stdout);
                let actual_run = execute(&actual);
                for result in [&expected_run, &actual_run] {
                    assert!(
                        result.status.success(),
                        "{}",
                        String::from_utf8_lossy(&result.stderr)
                    );
                }
                assert_eq!(
                    expected_run.stdout, expected_events,
                    "{python_version}, {optimization_level:?}"
                );
                assert_eq!(
                    actual_run.stdout, expected_run.stdout,
                    "{python_version}, {optimization_level:?}"
                );
                assert_eq!(
                    actual, native.stdout,
                    "{python_version}, {optimization_level:?}"
                );
            }
        }
    }
}

#[test]
fn optimized_match_guards_preserve_default_locations() {
    for (pattern, guard) in [("y", "__debug__"), ("_", "__debug__"), ("y", "False")] {
        let source = format!(
            "def f(x):\n    match x:\n        case {pattern} if {guard}:\n            return 1\n        case _:\n            return 2\n"
        );
        assert_matches_cpython(&source, "match_guard.py", &LEVELS);
    }

    for source in [
        r#"def f(x, flag, other):
    match x:
        case y if (
            __debug__
        ):
            raise RuntimeError("guard")
        case _:
            raise ValueError("default")
"#,
        r#"def f(x, manager, extra):
    try:
        trigger()
    except* ValueError as e:
        match x:
            case y if (
                __debug__
            ):
                raise RuntimeError("guard")
            case _:
                raise ValueError("default")
    return 9
"#,
        r#"def f(x, manager, extra):
    with manager:
        try:
            trigger()
        finally:
            match x:
                case y if (
                    __debug__
                ):
                    raise RuntimeError("guard")
                case _:
                    raise ValueError("default")
    return 9
"#,
    ] {
        assert_matches_cpython(source, "match_guard_control.py", &LEVELS);
    }
}

#[test]
fn elided_assertions_preserve_async_comprehension_return_location() {
    for source in [
        r"async def f(xs):
    result = [item async for item in xs]
    assert result
",
        r"async def f(xs):
    result = [
        item
        async for item in xs
    ]
    assert result
    assert len(result)
",
        r"async def f(xs):
    result = [item async for item in xs]
    assert result
    pass
",
        r"async def f(xs):
    assert xs
    result = [item async for item in xs]
",
        r"async def f(xs):
    result = [item for item in xs]
    assert result
",
        r"async def f(xs):
    result = await collect(xs)
    assert result
",
    ] {
        assert_matches_cpython(source, "implicit_return.py", &LEVELS);
    }
}

#[test]
fn elided_assertions_preserve_async_set_and_dict_return_locations() {
    for source in [
        r"async def f(xs):
    result = {item async for item in xs}
    assert result
",
        r"async def f(xs):
    result = {item: item async for item in xs}
    assert result
",
    ] {
        assert_matches_cpython(source, "implicit_return_comp.py", &LEVELS);
    }
}

#[test]
fn elided_assertions_preserve_sync_comprehension_return_locations() {
    for source in [
        r"def f(xs):
    result = [
        item
        for item in xs
        if item
    ]
    assert result
",
        r"def f(xs):
    result = {
        item: item
        for item in xs
    }
    assert result
",
        r"def f(xs):
    result = {
        item
        for item in xs
    }
    assert result
",
        r"def f(xs):
    result = [item for item in xs]
    assert result
",
    ] {
        assert_matches_cpython(source, "implicit_return_sync.py", &LEVELS);
    }
}

#[test]
fn elided_assertions_preserve_nested_generator_return_locations() {
    for source in [
        r"def f(xs):
    result = [x for x in xs if any(test(x, y) for y in xs)]
    assert result
",
        r"def f(xs):
    result = {
        x: sum(test(x, y) for y in xs)
        for x in xs
    }
    assert result
",
        r"def f(xs):
    result = [x for x in xs if any(test(y) for y in xs)]
    assert result
",
        r"def f(xs):
    result = [x for x in xs if any(test(x, y) for y in xs)]
    assert result
    pass
",
    ] {
        assert_matches_cpython(source, "implicit_return_nested_gen.py", &LEVELS);
    }
}

#[test]
fn async_comprehension_preserves_conditional_exception_owner() {
    const TRACE_CONDITION: &str = r"
import dis, marshal, sys

scope = {'work': lambda: None}
exec(marshal.loads(sys.stdin.buffer.read()), scope)
f = scope['f']
targets = [i.offset for i in dis.get_instructions(f)
           if i.opname == 'NOT_TAKEN' and i.positions.lineno == int(sys.argv[1])]
assert len(targets) == 1, targets
target = targets[0]
hits = []
def trace(frame, event, arg):
    if frame.f_code is f.__code__:
        frame.f_trace_opcodes = True
        if event == 'opcode' and frame.f_lasti == target:
            hits.append(frame.f_lasti)
            raise ValueError('injected')
    return trace
def drive(x):
    try:
        f(x, [('k', 1), ('j', 2)], {}).send(None)
    except StopIteration:
        return 'completed'
    except ValueError:
        return 'escaped'
    return 'suspended'
sys.settrace(trace)
warm = drive(False)
sys.settrace(None)
hits.clear()
sys.settrace(trace)
try:
    outcome = drive(True)
finally:
    sys.settrace(None)
assert warm == 'completed', warm
assert hits == [target], hits
print(outcome)
";
    let cases = [
        ("{**{k: 1 for k in xs}}", "", 4, false),
        ("[{k: 1 for k in xs}]", "", 4, false),
        ("[x, *[k for k in xs], x]", "", 4, false),
        ("({k: 1 for k in xs}, 1)", "", 4, false),
        ("(x, {k: 1 for k in xs})", "", 4, false),
        ("([x, *xs], [x, *[k for k in xs]])", "", 4, false),
        ("([x, *[k for k in xs]], [x, *xs])", "", 4, false),
        ("(*({k: 1 for k in xs},), x)", "", 4, false),
        ("([k for k in xs], (*xs, 1, 2))", "", 4, false),
        ("([k for k in xs], (*[1, 2], [x, *xs]))", "", 4, false),
        ("{**dict(xs)}", "", 4, true),
        ("{**{k: 1 for k in xs}}", "    pass\n", 5, false),
        ("{**{k: 1 for k in xs}}", "    pass; pass\n", 5, true),
        (
            "{**{k: 1 for k in xs}}",
            "    try:\n        work()\n    finally:\n        pass\n",
            8,
            false,
        ),
    ];
    for (expression, prefix, condition_line, protected_314) in cases {
        let source = format!(
            "async def f(x, xs, d):\n    v = {expression}\n{prefix}    try:\n        if x:\n            pass\n    except Exception:\n        pass\n"
        );
        for (index, (python_version, _, _)) in VERSIONS.iter().copied().enumerate() {
            for optimization_level in LEVELS {
                let native = oracle(
                    source.as_bytes(),
                    "case.py",
                    python_version,
                    optimization_level,
                );
                assert!(
                    native.status.success(),
                    "{}",
                    String::from_utf8_lossy(&native.stderr)
                );
                let options = CompileOptions {
                    python_version,
                    optimization_level,
                    ..CompileOptions::default()
                };
                let actual = compile_with_options(&source, "case.py", &options)
                    .unwrap()
                    .marshal();
                assert_eq!(
                    actual, native.stdout,
                    "{python_version}, {optimization_level:?}, {expression}"
                );
                if matches!(python_version, PythonVersion::Py314 | PythonVersion::Py315) {
                    let expected = if protected_314 && python_version == PythonVersion::Py314 {
                        &b"completed\n"[..]
                    } else {
                        &b"escaped\n"[..]
                    };
                    for code in [&native.stdout, &actual] {
                        let mut child = Command::new(ORACLES[index].get().unwrap())
                            .args(["-I", "-S", "-B", "-c", TRACE_CONDITION])
                            .arg(condition_line.to_string())
                            .stdin(Stdio::piped())
                            .stdout(Stdio::piped())
                            .stderr(Stdio::piped())
                            .spawn()
                            .unwrap();
                        child.stdin.take().unwrap().write_all(code).unwrap();
                        let result = child.wait_with_output().unwrap();
                        assert!(
                            result.status.success(),
                            "{}",
                            String::from_utf8_lossy(&result.stderr)
                        );
                        assert_eq!(
                            result.stdout, expected,
                            "{python_version}, {optimization_level:?}, {expression}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn comprehension_collection_continuations_preserve_exception_owners() {
    for expression in [
        "[x, *[k for k in xs], x]",
        "({k: 1 for k in xs}, 1)",
        "([x, *xs], [x, *[k for k in xs]])",
        "(*({k: 1 for k in xs},), x)",
        "([k for k in xs], (*[1, 2], [x, *xs]))",
    ] {
        let source = format!(
            "def f(x, xs, d):\n    v = {expression}\n    try:\n        if x:\n            pass\n    except Exception:\n        pass\n"
        );
        assert_matches_cpython(&source, "case.py", &LEVELS);
    }
}

#[test]
fn elided_assertion_identifiers_preserve_frozenset_identity() {
    const SOURCE: &str = r#"
def first():
    sink(items={"shared_member", "second_member", "third_member"})

def second():
    sink(extra=None, items={"shared_member", "second_member", "third_member"})

def discarded_assert():
    assert context.shared_member
"#;
    const TEXT_ORACLE: &str = r"
import marshal, sys
source = sys.stdin.buffer.read().decode('utf-8')
code = compile(source, sys.argv[2], 'exec', dont_inherit=True, optimize=int(sys.argv[1]))
sys.stdout.buffer.write(marshal.dumps(code))
";
    const IDENTITY_ORACLE: &str = r"
import marshal, sys, types
module = marshal.loads(sys.stdin.buffer.read())
functions = {c.co_name: c for c in module.co_consts if isinstance(c, types.CodeType)}
sets = [next(c for c in functions[name].co_consts if isinstance(c, frozenset))
        for name in ('first', 'second')]
assert sets[0] == sets[1]
assert (sets[0] is sets[1]) == (sys.argv[1] == 'shared')
";
    let cases = [
        (SOURCE.to_string(), false),
        (SOURCE.replace("    assert context.shared_member", "    pass"), true),
        (SOURCE.replace("    assert context.shared_member", "    context.shared_member"), false),
        (SOURCE.replace("extra=None, ", ""), true),
        (SOURCE.replace("\"shared_member\"", "\"Key_member\"").replace("context.shared_member", "context.\u{212a}ey_member"), false),
        (SOURCE.replace("assert context.shared_member", "assert (lambda: context.shared_member)()"), false),
        (SOURCE.replace("shared_member", "_Box__member").replace("def discarded_assert():\n    assert context._Box__member", "class Box:\n    def f(self):\n        return self.__member\n\n    def discarded_assert(self):\n        assert context._Box__member"), false),
        (SOURCE.replace("def discarded_assert():\n    assert context.shared_member", "assert context.shared_member"), false),
    ];
    for (source, shared) in cases {
        for (version, _, _) in VERSIONS {
            for level in [OptimizationLevel::One, OptimizationLevel::Two] {
                let options = CompileOptions {
                    python_version: version,
                    optimization_level: level,
                    ..CompileOptions::default()
                };
                let expected = oracle(source.as_bytes(), "case.py", version, level);
                assert!(
                    expected.status.success(),
                    "{}",
                    String::from_utf8_lossy(&expected.stderr)
                );
                let actual = compile_bytes_with_options(source.as_bytes(), "case.py", &options)
                    .unwrap()
                    .marshal();
                assert_eq!(actual, expected.stdout, "{version}, {level:?}, bytes");

                let index = VERSIONS
                    .iter()
                    .position(|(candidate, _, _)| *candidate == version)
                    .unwrap();
                let executable = ORACLES[index].get().expect("oracle identity was checked");
                let level_arg = if level == OptimizationLevel::One {
                    "1"
                } else {
                    "2"
                };
                let mut child = Command::new(executable)
                    .args([
                        "-I",
                        "-S",
                        "-X",
                        "utf8",
                        "-c",
                        TEXT_ORACLE,
                        level_arg,
                        "case.py",
                    ])
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .unwrap();
                child
                    .stdin
                    .take()
                    .unwrap()
                    .write_all(source.as_bytes())
                    .unwrap();
                let expected_text = child.wait_with_output().unwrap();
                assert!(
                    expected_text.status.success(),
                    "{}",
                    String::from_utf8_lossy(&expected_text.stderr)
                );
                let actual_text = compile_with_options(&source, "case.py", &options)
                    .unwrap()
                    .marshal();
                assert_eq!(
                    actual_text, expected_text.stdout,
                    "{version}, {level:?}, text"
                );

                for (label, payload) in [
                    ("native", expected.stdout.as_slice()),
                    ("serc", actual.as_slice()),
                ] {
                    let mut child = Command::new(executable)
                        .args([
                            "-I",
                            "-S",
                            "-X",
                            "utf8",
                            "-c",
                            IDENTITY_ORACLE,
                            if shared { "shared" } else { "distinct" },
                        ])
                        .stdin(Stdio::piped())
                        .stdout(Stdio::piped())
                        .stderr(Stdio::piped())
                        .spawn()
                        .unwrap();
                    child.stdin.take().unwrap().write_all(payload).unwrap();
                    let identity = child.wait_with_output().unwrap();
                    assert!(
                        identity.status.success(),
                        "{version}, {level:?}, {label}: {}",
                        String::from_utf8_lossy(&identity.stderr)
                    );
                }
            }
        }
    }
}

#[test]
fn elided_non_ascii_identifier_preserves_discarded_builders() {
    const PREFIX: &str = "def marker():\n    assert probe.élève\n\n";
    let discarded = [
        "if False:\n    def hidden(value):\n        return value + 1\n",
        "saved = (\"élève\",)\nif False:\n    class Hidden:\n        def method(self):\n            return (\"élève\",)\n",
        "if False:\n    async def hidden(items):\n        async for item in items:\n            try:\n                yield item\n            finally:\n                cleanup()\n",
    ];
    for body in discarded {
        assert_matches_cpython(&format!("{PREFIX}{body}"), "case.py", &LEVELS);
    }
}

#[test]
fn elided_assertions_after_conditional_comprehension_keep_return_locations() {
    for source in [
        "def f(x, y):\n    a = [v for v in x] if y else []\n    assert x\n",
        "def f(x, y):\n    a = {v for v in x} if y else y\n    assert x\n    assert y\n",
        "def f(x, y):\n    a = {v: v for v in x} if check(y) else []\n    assert x\n    assert y\n",
        "def f(x, y):\n    a = [v for v in x if v] if y else []\n    assert x\n",
        "def f(x, y):\n    a = [v + w for v in x for w in x] if y else []\n    assert x\n    assert y\n",
        "def f(x, y):\n    a = [(z := v) for v in x] if y else []\n    assert x\n",
        "def f(x, y):\n    a = [v for v in x] if y else [str(y)]\n    assert x\n",
        "def f(x, y):\n    a = [v for v in x] if y else []\n    assert (z := y)\n",
        "def f(x, y):\n    a = [(z := v) for v in x] if y else []\n    assert (z := y)\n",
        "def outer():\n    a = z = w = None\n    def f(x, y):\n        nonlocal a, z, w\n        a = [v for v in x] if y else []\n        assert (z := y)\n    return f\n",
        "def f(x, y, callback, manager):\n    with manager:\n        if x:\n            assert x\n        else:\n            assert y\n        callback(x)\n    a = [v for v in x] if y else []\n    assert x\n",
        "def f(x, y):\n    global a, z\n    a = [v for v in x] if y else []\n    assert x\n    assert y\n",
        "def f(x, y):\n    def inner():\n        return a, z\n    a = [(z := v) for v in x] if y else []\n    assert x\n    assert y\n",
        "def outer():\n    a = z = None\n    def f(x, y):\n        nonlocal a, z\n        a = [(z := v) for v in x] if y else []\n        assert x\n        assert y\n    return f\n",
        "def f(x, y):\n    try:\n        a = [v for v in x] if y else []\n        assert x\n    finally:\n        clean()\n",
        "def f(x, y, obj):\n    obj.a = [v for v in x] if y else []\n    assert x\n",
    ] {
        assert_matches_cpython_versions(
            source,
            "sub/conditional_comprehension_assert.py",
            &LEVELS,
            &[
                PythonVersion::Py312,
                PythonVersion::Py313,
                PythonVersion::Py314,
                PythonVersion::Py315,
            ],
        );
    }
}

#[test]
fn elided_assertions_after_terminal_comprehension_branches_match_cpython() {
    for source in [
        "def f(x, y, z, cm):\n    if x:\n        a = [i for i in y]\n    assert x\n",
        "def f(x, y, z, cm):\n    if x:\n        a = {i for i in y}\n    assert x\n",
        "def f(x, y, z, cm):\n    if x:\n        a = {i: i for i in y}\n    assert x\n",
        "def f(x, y, z, cm):\n    if x:\n        a = [(i, j) for i in y if i for j in y if j]\n    assert x\n",
        "def f(x, y, z, cm):\n    if x:\n        a = [(b := i) for i in y]\n    assert x\n",
        "def f(x, y, z, cm):\n    if x:\n        work()\n    else:\n        a = [i for i in y]\n    assert x\n",
        "def f(x, y, z, cm):\n    if x:\n        a = [i for i in y]\n    elif z:\n        a = {i for i in y}\n    else:\n        a = {i:i for i in y}\n    assert x\n",
        "def f(x, y, z, cm):\n    if x or z:\n        assert z\n    else:\n        a = [i for i in y]\n    assert x\n",
        "def f(x, y, z, cm):\n    if x:\n        a = [i for i in y]\n    else:\n        return x\n    assert x\n",
        "def f(x, y, z, cm):\n    if x:\n        a = [i for i in y]\n    else:\n        raise ValueError()\n    assert x\n",
        "def f(x, y):\n    global a\n    if x:\n        a = [i for i in y]\n    assert x\n",
        "def outer():\n    a = None\n    def f(x, y):\n        nonlocal a\n        if x:\n            a = [i for i in y]\n        assert x\n    return f\n",
        "def f(x, y, z, cm):\n    if x:\n        a, b = [i for i in y]\n    assert x\n",
        "def f(x, y, z, cm):\n    if 1 < x < z:\n        a = [i for i in y]\n    else:\n        pass\n    assert x\n",
        "def f(x, y, z, cm):\n    try:\n        if x:\n            a = [i for i in y]\n        assert x\n    finally:\n        work()\n",
        "def f(x, y, z, cm):\n    with cm:\n        if x:\n            a = [i for i in y]\n        assert x\n",
    ] {
        assert_matches_cpython(source, "sub/terminal_comp_assert.py", &LEVELS);
    }
}

#[test]
fn elided_assertions_preserve_terminal_branch_comprehension_returns() {
    for source in [
        r"def f(flag, xs):
    if flag:
        result = [x for x in xs]
        assert result
",
        r"async def f(flag, xs):
    if flag:
        result = [x for x in xs]
        assert result
",
        r"def f(flag, xs):
    if flag:
        result = {x for x in xs}
        assert result
",
        r"def f(flag, xs):
    if flag:
        result = {x: x for x in xs}
        assert result
",
        r"def f(flag, xs):
    if flag:
        result = [x for x in xs]
        assert result
    else:
        assert flag
",
        r"def f(flag, xs):
    if flag:
        assert flag
    else:
        result = [x for x in xs]
        assert result
",
        r"def f(flag, xs):
    if flag:
        result = [x for x in xs]
        assert result
        pass
",
        r"def f(flag, xs):
    try:
        if flag:
            result = [x for x in xs]
            assert result
    except LookupError:
        recover()
",
        r"def f(flag, xs):
    try:
        if flag:
            result = [x for x in xs]
            assert result
    finally:
        cleanup()
",
        r"def f(flag, xs, manager):
    with manager:
        if flag:
            result = [x for x in xs]
            assert result
",
    ] {
        assert_matches_cpython(source, "terminal_comprehension_assert.py", &LEVELS);
    }
}

#[test]
fn nested_conditional_except_before_finally_matches_cpython() {
    for source in [
        "def f(x, d):\n    try:\n        try:\n            if x:\n                pass\n        except Exception:\n            pass\n    finally:\n        d.clear()\n",
        "async def f(x, d):\n    try:\n        try:\n            if x:\n                work()\n        except:\n            pass\n    finally:\n        d.clear()\n",
        "def f(x, d):\n    try:\n        try:\n            if x:\n                pass\n        except Exception:\n            pass\n        else:\n            work()\n    finally:\n        d.clear()\n",
        "def f(x, d):\n    try:\n        if x:\n            pass\n    except Exception:\n        pass\n    d.clear()\n",
    ] {
        assert_matches_cpython(source, "case.py", &LEVELS);
    }
}

#[test]
fn chained_generator_loop_tail_preserves_stop_iteration_wrapper() {
    const SOURCE: &str = "def test(xs, y, z):\n    yield 0\n    for x in xs:\n        if x:\n            h()\n        elif x < y < z:\n            yield x\n        else:\n            assert z\n";
    const TRACE_ORACLE: &str = r"
import dis, marshal, sys
namespace = {'h': lambda: None}
exec(marshal.loads(sys.stdin.buffer.read()), namespace)
function = namespace['test']
instructions = list(dis.get_instructions(function))
targets = [instructions[i].offset for i in range(1, len(instructions))
           if instructions[i].opname == 'JUMP_BACKWARD'
           and instructions[i - 1].opname == 'POP_TOP'
           and instructions[i].positions.lineno == 6]
assert len(targets) == 1, targets
generator = function([0], -1, 3)
assert next(generator) == 0

def trace(frame, event, arg):
    if frame.f_code is function.__code__:
        frame.f_trace_opcodes = True
        if event == 'opcode' and frame.f_lasti == targets[0]:
            raise StopIteration('trace interruption')
    return trace

sys.settrace(trace)
try:
    next(generator)
except RuntimeError as exc:
    assert isinstance(exc.__cause__, StopIteration), exc.__cause__
else:
    raise AssertionError('exception was not propagated')
finally:
    sys.settrace(None)
";
    for version in [PythonVersion::Py314, PythonVersion::Py315] {
        for level in [OptimizationLevel::One, OptimizationLevel::Two] {
            let expected = oracle(SOURCE.as_bytes(), "generator_chain.py", version, level);
            assert!(expected.status.success(), "{version}, {level:?}");
            let actual = compile_with_options(
                SOURCE,
                "generator_chain.py",
                &CompileOptions {
                    python_version: version,
                    optimization_level: level,
                    ..CompileOptions::default()
                },
            )
            .unwrap()
            .marshal();
            let index = VERSIONS
                .iter()
                .position(|(candidate, _, _)| *candidate == version)
                .unwrap();
            for (label, payload) in [
                ("native", expected.stdout.as_slice()),
                ("serc", actual.as_slice()),
            ] {
                let mut child = Command::new(ORACLES[index].get().unwrap())
                    .args(["-I", "-S", "-X", "utf8", "-c", TRACE_ORACLE])
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .unwrap();
                child.stdin.take().unwrap().write_all(payload).unwrap();
                let result = child.wait_with_output().unwrap();
                assert!(
                    result.status.success(),
                    "{version}, {level:?}, {label}: {}",
                    String::from_utf8_lossy(&result.stderr)
                );
            }
        }
    }
}

#[test]
fn nested_chained_condition_in_generator_loop_tail_matches_cpython() {
    assert_matches_cpython_versions(
        "def test(xs, y, z):\n    yield 0\n    for x in xs:\n        if x:\n            if x < y < z:\n                yield x\n        else:\n            assert z\n",
        "tests/nested_generator_chain.py",
        &[OptimizationLevel::One, OptimizationLevel::Two],
        &[PythonVersion::Py314, PythonVersion::Py315],
    );
}

#[test]
fn protected_loop_elided_else_excludes_conditional_cleanup() {
    const SOURCE: &str = "def test(xs, manager, flag):\n    try:\n        for x in xs:\n            if x:\n                with manager as value:\n                    pass\n                    pass\n                    global marker\n            else:\n                assert flag\n    finally:\n        cleanup()\n    return\n";
    const TRACE_ORACLE: &str = r"
import contextlib, dis, marshal, sys
class Interrupted(Exception):
    pass

events = []
namespace = {'cleanup': lambda: events.append('cleanup')}
exec(marshal.loads(sys.stdin.buffer.read()), namespace)
function = namespace['test']
targets = [instruction.offset for instruction in dis.get_instructions(function)
           if instruction.positions.lineno == 4
           and instruction.opname.startswith('POP_JUMP')]
assert len(targets) == 1, targets
for value in (False, True):
    hit = False
    def trace(frame, event, arg):
        global hit
        if frame.f_code is function.__code__:
            frame.f_trace_opcodes = True
            if event == 'opcode' and frame.f_lasti == targets[0]:
                hit = True
                raise Interrupted()
        return trace
    sys.settrace(trace)
    try:
        function([value], contextlib.nullcontext(), True)
    except Interrupted:
        pass
    else:
        raise AssertionError('exception was not propagated')
    finally:
        sys.settrace(None)
    assert hit, 'conditional was not executed'
    assert not events, events
";
    for version in [PythonVersion::Py314, PythonVersion::Py315] {
        for level in [OptimizationLevel::One, OptimizationLevel::Two] {
            let expected = oracle(SOURCE.as_bytes(), "protected_loop.py", version, level);
            assert!(expected.status.success(), "{version}, {level:?}");
            let actual = compile_with_options(
                SOURCE,
                "protected_loop.py",
                &CompileOptions {
                    python_version: version,
                    optimization_level: level,
                    ..CompileOptions::default()
                },
            )
            .unwrap()
            .marshal();
            assert_eq!(actual, expected.stdout, "{version}, {level:?}");
            let index = VERSIONS
                .iter()
                .position(|(candidate, _, _)| *candidate == version)
                .unwrap();
            for (label, payload) in [
                ("native", expected.stdout.as_slice()),
                ("serc", actual.as_slice()),
            ] {
                let mut child = Command::new(ORACLES[index].get().unwrap())
                    .args(["-I", "-S", "-X", "utf8", "-c", TRACE_ORACLE])
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .unwrap();
                child.stdin.take().unwrap().write_all(payload).unwrap();
                let result = child.wait_with_output().unwrap();
                assert!(
                    result.status.success(),
                    "{version}, {level:?}, {label}: {}",
                    String::from_utf8_lossy(&result.stderr)
                );
            }
        }
    }
}

#[test]
fn elided_assert_after_nested_while_keeps_backedge_location() {
    for source in [
        r"def f(xs,x,y,cm):
    while xs:
        if x:
            while y:
                if y:
                    raise ValueError
            assert x
        else:
            break
",
        r"def f(xs,x,y,cm):
    while xs:
        if x:
            while y:
                if y:
                    return 1
            assert x
        else:
            continue
",
        r"def f(xs,x,y,cm):
    while xs:
        if x:
            while y:
                if y:
                    work()
            assert x
        else:
            work()
",
        r"def f(xs,x,y,cm):
    for z in xs:
        if x:
            while y:
                if y:
                    work()
            assert x
        else:
            return 1
",
        r"def f(xs,x,y):
    while xs:
        if x:
            while y:
                if y:
                    raise ValueError
            assert x
            assert y
        else:
            break
",
        r"def f(xs,x,y):
    while xs:
        if x:
            while y:
                if y:
                    raise ValueError
        else:
            break
",
        r"def f(xs,x,y):
    while xs:
        if x:
            while y:
                if y:
                    raise ValueError
            assert x
            pass
        else:
            break
",
        r"def f(xs,x,y):
    while xs:
        if x:
            while y:
                if y:
                    raise ValueError
            else:
                work()
            assert x
        else:
            break
",
        r"def f(xs,x,y,cm):
    while xs:
        if x:
            while y:
                try:
                    work()
                finally:
                    clean()
            assert x
        else:
            break
",
        r"def f(xs,x,y,cm):
    while xs:
        if x:
            while y:
                with cm:
                    work()
            assert x
        else:
            break
",
        r"def f(xs,x,y):
    while xs:
        if x:
            while (
                y()
            ):
                if x:
                    raise ValueError
            assert x
        else:
            break
",
        r"def f(xs,x,y,z):
    while xs:
        if x:
            while y:
                if x:
                    raise ValueError
            assert x
        elif z:
            work()
        else:
            break
",
    ] {
        assert_matches_cpython_versions(
            source,
            "sub/nested_while_elided.py",
            &LEVELS,
            &[
                PythonVersion::Py312,
                PythonVersion::Py313,
                PythonVersion::Py314,
                PythonVersion::Py315,
            ],
        );
    }
}

#[test]
fn nested_try_fallthrough_preserves_conditional_exception_owners() {
    const EXACT: &str = "def f(a,b,c,d):\n    while c:\n        try:\n            try:\n                if a:\n                    pass\n            except Exception:\n                pass\n        finally:\n            x = 0\n        break\n";
    const FRESH: &str = "def f(a,b,c,d):\n    while c:\n        try:\n            try:\n                if a:\n                    pass\n            except Exception:\n                pass\n        finally:\n            d.clear()\n        break\n";
    const RETAINED: &str = "def f(a,b,c,d):\n    while c:\n        try:\n            try:\n                pass; pass\n                if a:\n                    pass\n            except Exception:\n                pass\n        finally:\n            d.clear()\n        break\n";
    const TRACE_CONDITION: &str = r"
import dis, marshal, sys

events = []
class Injected(Exception):
    pass
class Clean:
    def clear(self):
        events.append('cleanup')
scope = {}
exec(marshal.loads(sys.stdin.buffer.read()), scope)
f = scope['f']
targets = [i.offset for i in dis.get_instructions(f)
           if i.opname == 'NOT_TAKEN' and i.positions.lineno == int(sys.argv[1])]
assert len(targets) == 1, targets
target = targets[0]
hits = []
monitor = sys.monitoring
monitor.use_tool_id(5, 'conditional-owner')
def inject(code, offset):
    if offset == target:
        hits.append(offset)
        raise Injected()
monitor.register_callback(5, monitor.events.INSTRUCTION, inject)
monitor.set_local_events(5, f.__code__, monitor.events.INSTRUCTION)
try:
    f(True, False, True, Clean())
    outcome = 'returned'
except Injected:
    outcome = 'escaped'
finally:
    monitor.set_local_events(5, f.__code__, 0)
    monitor.register_callback(5, monitor.events.INSTRUCTION, None)
    monitor.free_tool_id(5)
assert hits == [target], hits
print(outcome, events)
";

    assert_matches_cpython(EXACT, "fallthrough_nested_try.py", &LEVELS);
    for (source, line, expected) in [
        (FRESH, 5, &b"escaped []\n"[..]),
        (RETAINED, 6, &b"returned ['cleanup']\n"[..]),
    ] {
        for optimization_level in LEVELS {
            let native = oracle(
                source.as_bytes(),
                "fallthrough_nested_try.py",
                PythonVersion::Py314,
                optimization_level,
            );
            assert!(
                native.status.success(),
                "{}",
                String::from_utf8_lossy(&native.stderr)
            );
            let actual = compile_with_options(
                source,
                "fallthrough_nested_try.py",
                &CompileOptions {
                    python_version: PythonVersion::Py314,
                    optimization_level,
                    ..CompileOptions::default()
                },
            )
            .unwrap()
            .marshal();
            for code in [&native.stdout, &actual] {
                let mut child = Command::new(ORACLES[2].get().unwrap())
                    .args(["-I", "-S", "-B", "-c", TRACE_CONDITION])
                    .arg(line.to_string())
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .unwrap();
                child.stdin.take().unwrap().write_all(code).unwrap();
                let result = child.wait_with_output().unwrap();
                assert!(
                    result.status.success(),
                    "{}",
                    String::from_utf8_lossy(&result.stderr)
                );
                assert_eq!(
                    result.stdout, expected,
                    "{optimization_level:?}, line {line}"
                );
            }
        }
    }
}

#[test]
fn elided_match_case_assertions_keep_success_locations() {
    for source in [
        r"def f(x, y):
    match x:
        case False:
            work()
        case str():
            assert x
        case True:
            work()
",
        r"def f(x, y):
    match x:
        case False:
            work()
        case str(v):
            assert x
            assert y
        case True:
            work()
",
        r"def f(x, y):
    match x:
        case [1] | [2]:
            assert x
        case False:
            work()
        case True:
            work()
",
        r"def f(x, y):
    match x:
        case False:
            work()
        case 1:
            assert x
        case True:
            work()
",
        r"def f(x, y):
    match x:
        case False:
            work()
        case str():
            assert x
            pass
        case True:
            work()
",
        r"def f(x, y):
    match x:
        case False:
            work()
        case str() if y:
            assert (v := x)
        case True:
            work()
",
        r"def f(x, y):
    global v
    match x:
        case False:
            work()
        case str():
            assert (v := x)
            assert y
        case True:
            work()
",
    ] {
        assert_matches_cpython_versions(
            source,
            "sub/elided_match.py",
            &LEVELS,
            &[
                PythonVersion::Py312,
                PythonVersion::Py313,
                PythonVersion::Py314,
                PythonVersion::Py315,
            ],
        );
    }
}

#[test]
fn optimized_with_assertion_join_before_call_matches_cpython() {
    for condition in ["x", "not x", "x and y", "x < y", "f(x)", "x is None"] {
        let source = format!(
            "def f(x, y, callback, manager):\n    with manager:\n        if {condition}:\n            assert x\n        else:\n            assert y\n        callback(x)\n"
        );
        assert_matches_cpython_versions(
            &source,
            "optimized_with_join.py",
            &LEVELS,
            &[
                PythonVersion::Py312,
                PythonVersion::Py313,
                PythonVersion::Py314,
                PythonVersion::Py315,
            ],
        );
    }

    assert_matches_cpython_versions(
        "def f(x, y, callback, manager1, manager2):\n    with manager1, manager2:\n        if y:\n            if x:\n                assert x\n            else:\n                assert y\n            callback(x if y else x)\n",
        "optimized_with_nested_join.py",
        &LEVELS,
        &[PythonVersion::Py314],
    );
}

#[test]
fn nested_pass_exit_retains_enclosing_try_owner() {
    const SOURCE: &str = "def f(flag, items):\n    try:\n        try:\n            pass\n        except ValueError:\n            pass\n        work()\n    except Fault:\n        events.append('caught')\n";
    const TRACE: &str = r"
import dis, marshal, sys
class Fault(Exception):
    pass
events = []
scope = {'Fault': Fault, 'events': events}
exec(marshal.loads(sys.stdin.buffer.read()), scope)
f = scope['f']
targets = [i.offset for i in dis.get_instructions(f)
           if i.opname == 'NOP' and i.positions.lineno == 4]
assert len(targets) == 1, targets
target = targets[0]
hits = []
def trace(frame, event, arg):
    if frame.f_code is f.__code__:
        frame.f_trace_opcodes = True
        if event == 'opcode' and frame.f_lasti == target:
            hits.append(target)
            raise Fault()
    return trace
sys.settrace(trace)
try:
    f(True, [1])
finally:
    sys.settrace(None)
assert hits == [target], hits
assert events == ['caught'], events
";
    for (version, index) in [(PythonVersion::Py314, 2), (PythonVersion::Py315, 3)] {
        for optimization_level in LEVELS {
            let native = oracle(
                SOURCE.as_bytes(),
                "nested_pass_exit.py",
                version,
                optimization_level,
            );
            assert!(native.status.success());
            let actual = compile_with_options(
                SOURCE,
                "nested_pass_exit.py",
                &CompileOptions {
                    python_version: version,
                    optimization_level,
                    ..CompileOptions::default()
                },
            )
            .unwrap()
            .marshal();
            assert_eq!(actual, native.stdout, "{version} {optimization_level:?}");
            for payload in [&native.stdout, &actual] {
                let mut child = Command::new(ORACLES[index].get().unwrap())
                    .args(["-I", "-S", "-c", TRACE])
                    .stdin(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .unwrap();
                child.stdin.take().unwrap().write_all(payload).unwrap();
                let result = child.wait_with_output().unwrap();
                assert!(
                    result.status.success(),
                    "{version} {optimization_level:?}: {}",
                    String::from_utf8_lossy(&result.stderr)
                );
            }
        }
    }
}

#[test]
fn nested_pass_exit_successors_match_cpython() {
    for successor in ["out = 1", "del item", "if flag:\n            assert flag"] {
        let source = format!(
            "def f(flag, item):\n    try:\n        try:\n            pass\n        except ValueError:\n            pass\n        {successor}\n    except Fault:\n        events.append('caught')\n"
        );
        assert_matches_cpython_versions(
            &source,
            "nested_pass_successor.py",
            &LEVELS,
            &[PythonVersion::Py314, PythonVersion::Py315],
        );
    }

    assert_matches_cpython_versions(
        "def f():\n    try:\n        try: pass\n        except ValueError:\n            pass\n        work()\n    except Fault:\n        events.append('caught')\n",
        "nested_same_line_pass.py",
        &LEVELS,
        &[PythonVersion::Py314, PythonVersion::Py315],
    );
}

#[test]
fn nonterminal_pass_finally_preserves_handler_for_all_targets() {
    for (function, trailer) in [
        ("def", ""),
        ("async def", ""),
        ("def", "    yield 1\n"),
        ("async def", "    yield 1\n"),
    ] {
        let source = format!(
            "{function} f():\n    try:\n        try:\n            work()\n        finally:\n            pass\n        after()\n    except:\n        recover()\n{trailer}"
        );
        assert_matches_cpython(&source, "nonterminal_pass_finally.py", &LEVELS);
    }
}

#[test]
fn optimized_with_assertion_join_before_noncall_matches_cpython() {
    for successor in [
        "",
        "        value = 1\n",
        "        del flag\n",
        "        return flag\n",
        "        assert flag\n",
        "        if flag:\n            value = 1\n",
    ] {
        let source = format!(
            "def f(flag, manager):\n    with manager:\n        if flag:\n            assert flag\n        else:\n            assert flag\n{successor}"
        );
        assert_matches_cpython_versions(
            &source,
            "optimized_with_join_noncall.py",
            &LEVELS,
            &[PythonVersion::Py314],
        );
    }
    // The short-circuit edge also reaches this join and must not retain the NOP.
    assert_matches_cpython_versions(
        "def f(a, b, manager):\n    with manager:\n        if a or b:\n            assert a\n        else:\n            assert b\n        value = a\n",
        "optimized_with_shared_join.py",
        &LEVELS,
        &[PythonVersion::Py314],
    );
}

#[test]
fn optimized_with_assertion_join_before_noncall_keeps_manager_owner() {
    const SOURCE: &str = r"
def terminal(flag):
    with manager('terminal'):
        if flag:
            assert flag
        else:
            assert flag

def assigned(flag):
    with manager('outer'), manager('inner'):
        if flag: assert flag
        else: assert flag
        value = 1

def deleted(flag):
    with manager('delete'):
        if flag:
            assert flag
        else:
            assert flag
        del flag
";
    const TRACE: &str = r"
import dis, marshal, sys

events = []
class Fault(Exception):
    pass
class Manager:
    def __init__(self, name):
        self.name = name
    def __enter__(self):
        events.append((self.name, 'enter'))
    def __exit__(self, typ, value, tb):
        events.append((self.name, 'exit', typ.__name__ if typ else None))
        return False
scope = {'manager': Manager}
exec(marshal.loads(sys.stdin.buffer.read()), scope)
for name, expected in [
    ('terminal', [('terminal', 'enter'), 'hit', ('terminal', 'exit', 'Fault')]),
    ('assigned', [('outer', 'enter'), ('inner', 'enter'), 'hit',
                  ('inner', 'exit', 'Fault'), ('outer', 'exit', 'Fault')]),
    ('deleted', [('delete', 'enter'), 'hit', ('delete', 'exit', 'Fault')]),
]:
    fn = scope[name]
    instructions = list(dis.get_instructions(fn))
    offsets = [b.offset for a, b in zip(instructions, instructions[1:])
               if a.opname == 'NOT_TAKEN' and b.opname == 'NOP']
    assert len(offsets) == 1, (name, offsets)
    events.clear()
    def trace(frame, event, arg):
        if frame.f_code is fn.__code__:
            frame.f_trace_opcodes = True
            if event == 'opcode' and frame.f_lasti == offsets[0]:
                events.append('hit')
                raise Fault()
        return trace
    sys.settrace(trace)
    try:
        try:
            fn(True)
            raise AssertionError('injection did not escape')
        except Fault:
            pass
    finally:
        sys.settrace(None)
    assert events == expected, (name, events, expected)
";
    for level in [OptimizationLevel::One, OptimizationLevel::Two] {
        let native = oracle(
            SOURCE.as_bytes(),
            "optimized_with_join_owner.py",
            PythonVersion::Py314,
            level,
        );
        assert!(native.status.success());
        let actual = compile_with_options(
            SOURCE,
            "optimized_with_join_owner.py",
            &CompileOptions {
                python_version: PythonVersion::Py314,
                optimization_level: level,
                ..CompileOptions::default()
            },
        )
        .unwrap()
        .marshal();
        assert_eq!(actual, native.stdout);
        let index = VERSIONS
            .iter()
            .position(|(version, _, _)| *version == PythonVersion::Py314)
            .unwrap();
        for payload in [&native.stdout, &actual] {
            let mut child = Command::new(ORACLES[index].get().unwrap())
                .args(["-I", "-S", "-c", TRACE])
                .stdin(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            child.stdin.take().unwrap().write_all(payload).unwrap();
            let result = child.wait_with_output().unwrap();
            assert!(
                result.status.success(),
                "{level:?}: {}",
                String::from_utf8_lossy(&result.stderr)
            );
        }
    }
}

#[test]
fn protected_with_elided_if_keeps_manager_join_and_exception_owner() {
    const SOURCE: &str = r"
def f(flag, manager):
    try:
        with manager:
            if flag:
                assert flag
            else:
                assert not flag
        work()
    except Fault:
        recover()

def with_successor(flag, manager):
    try:
        with manager:
            if flag:
                assert flag
            else:
                assert not flag
            work()
    except Fault:
        recover()

def reverse(flag, manager):
    with manager:
        try:
            if flag:
                assert flag
            else:
                assert flag
        except Fault:
            recover()

def nested_reverse(flag, manager):
    with manager:
        try:
            try:
                if flag:
                    assert flag
                else:
                    assert flag
            except Fault:
                recover()
        except TypeError:
            work()

def except_star(flag, manager):
    try:
        with manager:
            if flag:
                assert flag
            else:
                assert not flag
    except* Fault:
        recover()
";
    const TRACE: &str = r"
import dis, marshal, sys

events = []
class Fault(Exception):
    pass
class Manager:
    def __enter__(self):
        events.append('enter')
    def __exit__(self, typ, value, tb):
        events.append(('exit', typ.__name__ if typ else None))
        return False
scope = {'Fault': Fault, 'work': lambda: events.append('work'),
         'recover': lambda: events.append('recover')}
exec(marshal.loads(sys.stdin.buffer.read()), scope)
for name, flag, expected in (
    ('f', True, ['enter', 'hit', ('exit', 'Fault'), 'recover']),
    ('with_successor', True, ['enter', 'hit', ('exit', 'Fault'), 'recover']),
    ('reverse', False, ['enter', 'hit', ('exit', 'Fault'), 'escaped']),
    ('nested_reverse', False, ['enter', 'hit', ('exit', 'Fault'), 'escaped']),
    ('except_star', True, ['enter', 'hit', ('exit', 'Fault'), 'recover']),
):
    f = scope[name]
    events.clear()
    instructions = list(dis.get_instructions(f))
    line = next(i.positions.lineno for i in instructions if i.opname == 'POP_JUMP_IF_FALSE')
    offsets = [i.offset for i in instructions if i.opname == 'NOP' and i.positions.lineno == line]
    assert len(offsets) == 1, (name, offsets)
    def trace(frame, event, arg):
        if frame.f_code is f.__code__:
            frame.f_trace_opcodes = True
            if event == 'opcode' and frame.f_lasti == offsets[0]:
                events.append('hit')
                raise Fault()
        return trace
    sys.settrace(trace)
    try:
        try:
            f(flag, Manager())
        except Fault:
            events.append('escaped')
    finally:
        sys.settrace(None)
    assert events == expected, (name, events, expected)
";
    for version in [PythonVersion::Py314, PythonVersion::Py315] {
        assert_matches_cpython_versions(SOURCE, "protected_with_join.py", &LEVELS, &[version]);
        for level in [OptimizationLevel::One, OptimizationLevel::Two] {
            let native = oracle(SOURCE.as_bytes(), "protected_with_join.py", version, level);
            assert!(native.status.success());
            let actual = compile_with_options(
                SOURCE,
                "protected_with_join.py",
                &CompileOptions {
                    python_version: version,
                    optimization_level: level,
                    ..CompileOptions::default()
                },
            )
            .unwrap()
            .marshal();
            assert_eq!(actual, native.stdout);
            let index = VERSIONS
                .iter()
                .position(|(candidate, _, _)| *candidate == version)
                .unwrap();
            for payload in [&native.stdout, &actual] {
                let mut child = Command::new(ORACLES[index].get().unwrap())
                    .args(["-I", "-S", "-c", TRACE])
                    .stdin(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .unwrap();
                child.stdin.take().unwrap().write_all(payload).unwrap();
                let result = child.wait_with_output().unwrap();
                assert!(
                    result.status.success(),
                    "{version:?} {level:?}: {}",
                    String::from_utf8_lossy(&result.stderr)
                );
            }
        }
    }
}
