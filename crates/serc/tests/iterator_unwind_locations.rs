use std::io::Write as _;
use std::process::{Command, Stdio};

use serc::{CompileOptions, PythonVersion, compile_bytes_with_options, compile_with_options};

#[test]
fn copied_handler_cleanup_keeps_retired_iterator_unwinds_unlocated() {
    let oracle = r"
import dis, importlib.util, marshal, sys, types
assert sys.implementation.name == 'cpython'
assert sys.version.split()[0] == sys.argv[1]
assert importlib.util.MAGIC_NUMBER.hex() == sys.argv[2]

def iterator_cleanup(code):
    function = next(value for value in code.co_consts
                    if isinstance(value, types.CodeType) and value.co_name == 'f')
    instructions = list(dis.get_instructions(function))
    finish = next(index for index, instruction in enumerate(instructions)
                  if instruction.opname == 'LOAD_GLOBAL' and instruction.argval == 'finish')
    call = next(index for index in range(finish, len(instructions))
                if instructions[index].opname == 'CALL')
    assert instructions[call + 1].opname == 'POP_TOP'
    slots = 3 if sys.version_info >= (3, 15) else 2
    cleanup = instructions[call + 2 + slots:call + 2 + 2 * slots]
    assert [instruction.opname for instruction in cleanup] == (
        ['SWAP'] + ['POP_TOP'] * (slots - 1))
    return function.co_code, [(instruction.opname, instruction.arg,
                               tuple(instruction.positions)) for instruction in cleanup]

expected_code, expected = iterator_cleanup(compile(
    sys.argv[3], 'iterator_unwind_locations.py', 'exec', dont_inherit=True, optimize=0))
assert all(record[2] == (None, None, None, None) for record in expected)
actual_code, actual = iterator_cleanup(marshal.loads(sys.stdin.buffer.read()))
assert actual_code == expected_code
assert actual == expected, (actual, expected)
";
    for version in [PythonVersion::Py314, PythonVersion::Py315] {
        let target = version.target();
        let (variable, fallback) = match version {
            PythonVersion::Py314 => ("SERC_TEST_PYTHON", "python3.14"),
            PythonVersion::Py315 => ("SERC_TEST_PYTHON_315", "python3.15"),
            _ => unreachable!(),
        };
        let python = std::env::var_os(variable).unwrap_or_else(|| fallback.into());
        let options = CompileOptions {
            python_version: version,
            ..CompileOptions::default()
        };
        for (scope, prefix) in [("def", ""), ("async def", ""), ("def", "    yield value\n")] {
            for returned in ["value", "value.result"] {
                let source = format!(
                    "{scope} f(value, flag, manager, values):
{prefix}    try:
        with manager, other_manager():
            for first in values:
                for second in values:
                    try:
                        for third in values:
                            if flag:
                                return {returned}
                    except Error:
                        raise
                    finally:
                        state = prepare(value) if flag else None
                        stage = 'first'
                        try:
                            if state is not None:
                                cleanup(state)
                            stage = 'second'
                            finish(value)
                        except Error as error:
                            value.failed = True
                            if not flag:
                                raise Other from error
    except Exception:
        raise
    finally:
        last()
"
                );
                // The inner iterator has already ended these region collectors.
                // The outermost iterator still needs its own unlocated boundary.
                for compiled in [
                    compile_with_options(&source, "iterator_unwind_locations.py", &options)
                        .unwrap(),
                    compile_bytes_with_options(
                        source.as_bytes(),
                        "iterator_unwind_locations.py",
                        &options,
                    )
                    .unwrap(),
                ] {
                    let mut child = Command::new(&python)
                        .args(["-B", "-I", "-S", "-X", "utf8", "-c", oracle])
                        .args([
                            target.tag.trim_start_matches('v'),
                            &target.magic_hex(),
                            &source,
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
                        .write_all(&compiled.marshal())
                        .unwrap();
                    let output = child.wait_with_output().unwrap();
                    assert!(
                        output.status.success(),
                        "{version:?}: {source}\n{}",
                        String::from_utf8_lossy(&output.stderr)
                    );
                }
            }
        }
    }
}

#[test]
fn pending_finalizers_preserve_nested_iterator_false_exit_positions() {
    let cleanup = "if first is not None:\n    try:\n        await first.close()\n    except Exception as error:\n        report(error)\nif second is not None:\n    try:\n        await second.close()\n    except Exception as error:\n        report(error)";
    let oracle = r"
import dis, importlib.util, marshal, sys, types
assert sys.implementation.name == 'cpython'
assert sys.version.split()[0] == sys.argv[1]
assert importlib.util.MAGIC_NUMBER.hex() == sys.argv[2]

def false_exit(code):
    function = next(value for value in code.co_consts
                    if isinstance(value, types.CodeType) and value.co_name == 'f')
    instructions = list(dis.get_instructions(function))
    load = next(index for index, instruction in enumerate(instructions)
                if instruction.argval == 'second' and instruction.opname.startswith('LOAD_FAST'))
    condition = instructions[load + 1]
    assert condition.opname == 'POP_JUMP_IF_NONE'
    target = next(index for index, instruction in enumerate(instructions)
                  if instruction.offset == condition.argval)
    route = []
    for instruction in instructions[target:]:
        route.append((instruction.opname, instruction.arg, tuple(instruction.positions)))
        if instruction.opname == 'RETURN_VALUE':
            return route
        assert instruction.opname in ('SWAP', 'POP_TOP')
    raise AssertionError('false exit has no return')

expected = false_exit(compile(sys.argv[3], 'iterator_unwind_locations.py', 'exec',
                              dont_inherit=True, optimize=0))
actual = false_exit(marshal.loads(sys.stdin.buffer.read()))
assert actual == expected, (actual, expected)
";
    for version in [PythonVersion::Py314, PythonVersion::Py315] {
        let target = version.target();
        let (variable, fallback) = match version {
            PythonVersion::Py314 => ("SERC_TEST_PYTHON", "python3.14"),
            PythonVersion::Py315 => ("SERC_TEST_PYTHON_315", "python3.15"),
            _ => unreachable!(),
        };
        let python = std::env::var_os(variable).unwrap_or_else(|| fallback.into());
        let options = CompileOptions {
            python_version: version,
            ..CompileOptions::default()
        };
        for try_inside_both_loops in [false, true] {
            for returned in ["item", "make(item)"] {
                let mut body = format!(
                    "try:\n    return {returned}\nfinally:\n    {}",
                    cleanup.replace('\n', "\n    ")
                );
                if try_inside_both_loops {
                    body = format!(
                        "try:\n    {}\nexcept Exception:\n    raise",
                        body.replace('\n', "\n    ")
                    );
                }
                body = format!("for item in items:\n    {}", body.replace('\n', "\n    "));
                if !try_inside_both_loops {
                    body = format!(
                        "try:\n    {}\nexcept Exception:\n    raise",
                        body.replace('\n', "\n    ")
                    );
                }
                body = format!("for outer in items:\n    {}", body.replace('\n', "\n    "));
                let source = format!(
                    "async def f(items, first, second):\n    {}\n",
                    body.replace('\n', "\n    ")
                );
                // Follow the actual guard-false target so another copied return
                // cannot mask a missing location on this specific unwind path.
                for compiled in [
                    compile_with_options(&source, "iterator_unwind_locations.py", &options)
                        .unwrap(),
                    compile_bytes_with_options(
                        source.as_bytes(),
                        "iterator_unwind_locations.py",
                        &options,
                    )
                    .unwrap(),
                ] {
                    let mut child = Command::new(&python)
                        .args(["-B", "-I", "-S", "-X", "utf8", "-c", oracle])
                        .args([
                            target.tag.trim_start_matches('v'),
                            &target.magic_hex(),
                            &source,
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
                        .write_all(&compiled.marshal())
                        .unwrap();
                    let output = child.wait_with_output().unwrap();
                    assert!(
                        output.status.success(),
                        "{version:?}: {source}\n{}",
                        String::from_utf8_lossy(&output.stderr)
                    );
                }
            }
        }
    }
}

#[test]
fn guarded_handler_returns_keep_unowned_condition_cells() {
    let oracle = r"
import ast, dis, importlib.util, marshal, sys, types
assert sys.implementation.name == 'cpython'
assert sys.version.split()[0] == sys.argv[1]
assert importlib.util.MAGIC_NUMBER.hex() == sys.argv[2]

def copied_cleanup(code):
    function = next(value for value in code.co_consts
                    if isinstance(value, types.CodeType) and value.co_name == 'f')
    instructions = list(dis.get_instructions(function))
    guard = next(node for node in ast.walk(ast.parse(sys.argv[3]))
                 if isinstance(node, ast.If) and isinstance(node.test, ast.Name)
                 and node.test.id == 'flag' and isinstance(node.body[0], ast.Return))
    flag = next(index for index, instruction in enumerate(instructions)
                if instruction.opname.startswith('LOAD_FAST') and instruction.argval == 'flag'
                and instruction.positions.lineno == guard.lineno)
    branch = next(instruction for instruction in instructions[flag + 1:]
                  if instruction.opname == 'POP_JUMP_IF_FALSE')
    route = [instruction for instruction in instructions
             if branch.offset < instruction.offset < branch.argval]
    assert route[-1].opname == 'RETURN_VALUE'
    offsets = {instruction.offset: index for index, instruction in enumerate(route)}
    normalized = [(instruction.opname,
                   offsets[instruction.argval] if instruction.opcode in dis.hasjump
                   else instruction.arg) for instruction in route]
    condition = next(index for index, instruction in enumerate(route)
                     if instruction.opname in ('POP_JUMP_IF_NONE', 'POP_JUMP_IF_FALSE'))
    marker = route[condition + 1]
    assert marker.opname == 'NOT_TAKEN'
    owner = next((entry for entry in dis.Bytecode(function).exception_entries
                  if entry.start <= marker.offset < entry.end), None)
    return normalized, owner

expected, expected_owner = copied_cleanup(compile(
    sys.argv[3], 'iterator_unwind_locations.py', 'exec', dont_inherit=True, optimize=0))
actual, actual_owner = copied_cleanup(marshal.loads(sys.stdin.buffer.read()))
assert actual == expected, (actual, expected)
assert expected_owner is None, expected_owner
assert actual_owner is None, actual_owner
";
    let version = PythonVersion::Py314;
    let target = version.target();
    let python = std::env::var_os("SERC_TEST_PYTHON").unwrap_or_else(|| "python3.14".into());
    let options = CompileOptions {
        python_version: version,
        ..CompileOptions::default()
    };
    for (prefix, yield_statement) in [("", ""), ("async ", ""), ("", "    yield value\n")] {
        let source = format!(
            "{prefix}def f(flag, value, items):
{yield_statement}    try:
        for item in items:
            try:
                work(value)
            except Error:
                if flag:
                    return work(value)
                recover(value)
                break
            finally:
                if value is not None:
                    cleanup(value)
    finally:
        finish(value)
"
        );
        // Compare the copied return path, whose branch destinations align even
        // when the later break cleanup has a separate native layout difference.
        let nested_source = format!(
            "{prefix}def f(flag, value, items):
{yield_statement}    try:
        try:
            work(value)
        except Error:
            try:
                work(value)
            except InnerError:
                if flag:
                    return True
                recover(value)
        finally:
            if flag:
                cleanup(value)
    except OuterError:
        return False
"
        );
        for source in [source, nested_source] {
            for compiled in [
                compile_with_options(&source, "iterator_unwind_locations.py", &options).unwrap(),
                compile_bytes_with_options(
                    source.as_bytes(),
                    "iterator_unwind_locations.py",
                    &options,
                )
                .unwrap(),
            ] {
                let mut child = Command::new(&python)
                    .args(["-B", "-I", "-S", "-X", "utf8", "-c", oracle])
                    .args([
                        target.tag.trim_start_matches('v'),
                        &target.magic_hex(),
                        &source,
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
                    .write_all(&compiled.marshal())
                    .unwrap();
                let output = child.wait_with_output().unwrap();
                assert!(
                    output.status.success(),
                    "{version:?}: {source}\n{}",
                    String::from_utf8_lossy(&output.stderr)
                );
            }
        }
    }
}
