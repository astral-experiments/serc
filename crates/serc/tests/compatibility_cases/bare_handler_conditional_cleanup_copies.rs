use crate::common;
use crate::common::indent;

#[test]
fn matches_cpython_bare_handler_conditional_cleanup_copies() {
    for condition in [
        "flag",
        "flag and running",
        "flag or running",
        "value is None",
    ] {
        for arm in ["consume()", "result = value"] {
            for successor in ["finish()", "return value"] {
                let source = format!(
                    "def f(items, flag, running, manager, value):\n    try:\n        step()\n    except Error:\n        if {condition}:\n            {arm}\n        else:\n            other()\n    {successor}\n"
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "bare_handler_conditional_cleanup_copies.py",
                );
            }
        }
    }
}

#[test]
fn matches_cpython_folded_bare_handler_cleanup_return() {
    for arm in ["consume()", "result = value", "pass", "assert flag"] {
        let source = format!(
            "def f(items, flag, running, manager, value):\n    try:\n        step()\n    except Error:\n        if False:\n            {arm}\n        else:\n            other()\n    return value\n"
        );
        common::assert_matches_cpython_marshal(&source, "folded_bare_handler_cleanup_return.py");
    }
}

#[test]
fn matches_cpython_nested_handler_conditional_cleanup_joins() {
    for depth in [1, 2] {
        for inner_handler in ["Error", "Error as inner"] {
            for arm in ["value = 1", "effect()"] {
                let mut body = format!(
                    "try:\n    if flag:\n        {arm}\nexcept {inner_handler}:\n    pass\n"
                );
                for index in 0..depth {
                    body = format!(
                        "try:\n    first()\nexcept Error as error{index}:\n{}",
                        indent(&body)
                    );
                }
                let source = format!("def f(flag):\n{}    after()\n", indent(&body));
                common::assert_matches_cpython_marshal(
                    &source,
                    "nested_handler_conditional_cleanup_joins.py",
                );
            }
        }
    }
}

#[test]
fn matches_cpython_authored_pass_cleanup_after_copying() {
    for handler in ["Error", "Error as error"] {
        for body in [
            "if a:\n    pass\nelse:\n    raise\n",
            "if a or b:\n    pass\nelse:\n    raise\n",
            "if a and b:\n    pass\nelse:\n    raise\n",
            "prepare()\nif a or b:\n    pass\nelse:\n    raise\n",
            "if a or b:\n    prepare()\n    pass\nelse:\n    raise\n",
            "if a or b:\n    raise\nelse:\n    pass\n",
        ] {
            let handler_body = format!("try:\n    work()\nexcept {handler}:\n{}", indent(body));
            for body in [
                handler_body.clone(),
                format!("for value in values:\n{}", indent(&handler_body)),
                format!("try:\n    initial()\nfinally:\n{}", indent(&handler_body)),
            ] {
                let source = format!(
                    "def f(a, b, value, values):\n{}    return value\n",
                    indent(&body)
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "authored_pass_cleanup_after_copying.py",
                );
            }
        }
    }
}

#[test]
fn preserves_exceptional_cleanup_copies_through_forward_joins() {
    // These nested finalizers retain unrelated layout and source-position differences. Preserve the
    // native false-branch locations and the enclosing cleanup's exception owner while fixing
    // the ordinary continuation in the normal finalizer copy.
    const ORACLE: &str = r#"
import dis, marshal, pathlib, sys, types
ours = marshal.loads(pathlib.Path(sys.argv[1]).read_bytes())
reference = compile(pathlib.Path(sys.argv[2]).read_text(), "exceptional_cleanup_forward_join.py", "exec", dont_inherit=True)
def trace(module):
    code = next(value for value in module.co_consts if isinstance(value, types.CodeType))
    instructions = list(dis.get_instructions(code))
    index = {instruction.offset: i for i, instruction in enumerate(instructions)}
    entries = list(dis.Bytecode(code).exception_entries)
    branch = [i for i in instructions if i.opname == "POP_JUMP_IF_FALSE" and i.positions.lineno == 12][-1]
    cursor = index[branch.argval]
    seen = set()
    result = []
    while cursor not in seen:
        seen.add(cursor)
        instruction = instructions[cursor]
        if instruction.opname in ("JUMP_FORWARD", "JUMP_BACKWARD", "JUMP_BACKWARD_NO_INTERRUPT"):
            cursor = index[instruction.argval]
            continue
        entry = next((entry for entry in entries if entry.start <= instruction.offset < entry.end), None)
        owner = None
        if entry is not None:
            handler = instructions[index[entry.target]:index[entry.target] + 3]
            owner = (entry.depth, entry.lasti, tuple((i.opname, i.arg) for i in handler))
        result.append((instruction.opname, instruction.argval, tuple(instruction.positions), owner))
        if instruction.opname == "RERAISE":
            return result
        cursor += 1
    raise AssertionError("cleanup path did not reach RERAISE")
actual, expected = trace(ours), trace(reference)
assert len(actual) == len(expected) == 9
assert [i[:3] for i in actual[:4]] == [i[:3] for i in expected[:4]], "false-branch cleanup locations differ"
assert [(i[0], i[1], i[3]) for i in actual[4:]] == [(i[0], i[1], i[3]) for i in expected[4:]], "outer cleanup instructions or owners differ"
"#;
    for arm in ["value = other", "effect(value)"] {
        let source = format!(
            "def f(flag, value, other, items):\n    try:\n        initial()\n    finally:\n        try:\n            first()\n        except Error as outer:\n            try:\n                second()\n            except Error as middle:\n                try:\n                    if flag:\n                        {arm}\n                except Error as inner:\n                    recover(value)\n    return value\n"
        );
        let directory = tempfile::tempdir().unwrap();
        let source_path = directory.path().join("source.py");
        let marshal_path = directory.path().join("ours.marshal");
        std::fs::write(&source_path, &source).unwrap();
        std::fs::write(
            &marshal_path,
            serc::compile(&source, "exceptional_cleanup_forward_join.py")
                .unwrap()
                .marshal(),
        )
        .unwrap();
        let output = common::python_3145_command()
            .args(["-c", ORACLE])
            .arg(&marshal_path)
            .arg(&source_path)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn matches_cpython_exceptional_finally_condition_fallthrough_owners() {
    for handler in ["except:", "except Exception:", "except Exception as error:"] {
        let cleanup = format!("try:\n    if keep:\n        work()\n{handler}\n    pass\n");
        for cleanup in [
            cleanup.clone(),
            format!("if container:\n{}", indent(&cleanup)),
        ] {
            let source = format!(
                "def f(container, keep):\n    try:\n        work()\n    finally:\n{}",
                indent(&indent(&cleanup))
            );
            common::assert_matches_cpython_marshal(
                &source,
                "exceptional_finally_condition_fallthrough_owners.py",
            );
        }
    }
}

#[test]
fn preserves_owned_outer_finally_marker_before_fresh_inner_marker() {
    const ORACLE: &str = r#"
import dis, marshal, pathlib, sys, types
ours = marshal.loads(pathlib.Path(sys.argv[1]).read_bytes())
reference = compile(pathlib.Path(sys.argv[2]).read_text(), "finally_marker_owners.py", "exec", dont_inherit=True)
def marker_owners(module):
    code = next(value for value in module.co_consts if isinstance(value, types.CodeType))
    entries = list(dis.Bytecode(code).exception_entries)
    result = []
    for instruction in dis.get_instructions(code):
        if instruction.opname != "NOT_TAKEN":
            continue
        entry = next((entry for entry in entries if entry.start <= instruction.offset < entry.end), None)
        owner = None if entry is None else (entry.target, entry.depth, entry.lasti)
        result.append((tuple(instruction.positions), owner))
    return result
actual, expected = marker_owners(ours), marker_owners(reference)
assert len(actual) == len(expected) == 4
assert expected[2][1] is not None, "outer marker must reuse the owned removed SETUP slot"
assert expected[3][1] is None, "inner marker must use a fresh slot after the retained SETUP"
assert actual == expected, (actual, expected)
"#;
    let source = "def f(container, keep):\n    try:\n        work()\n    finally:\n        if container:\n            try:\n                if keep:\n                    work()\n            except:\n                pass\n";
    let directory = tempfile::tempdir().unwrap();
    let source_path = directory.path().join("source.py");
    let marshal_path = directory.path().join("ours.marshal");
    std::fs::write(&source_path, source).unwrap();
    std::fs::write(
        &marshal_path,
        serc::compile(source, "finally_marker_owners.py")
            .unwrap()
            .marshal(),
    )
    .unwrap();
    let output = common::python_3145_command()
        .args(["-c", ORACLE])
        .arg(&marshal_path)
        .arg(&source_path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn anonymous_handler_conditional_backedges_copy_cleanup() {
    for (filename, source) in [
        (
            "generator_handler_conditional_break_copy_target.py",
            r"def collect(manager, ready, work, pending, buffered):
    with manager():
        while True:
            yield None
            if ready():
                try:
                    work()
                except LookupError:
                    if pending() == 0 and buffered() == 0:
                        break
            else:
                work()
",
        ),
        (
            "generator_handler_single_break_copy_control.py",
            r"def collect(manager, ready, work, pending, buffered):
    with manager():
        while True:
            yield None
            if ready():
                try:
                    work()
                except LookupError:
                    if pending() == 0:
                        break
            else:
                work()
",
        ),
        (
            "manager_handler_conditional_tuple_copy_target.py",
            r"def consume(manager, items, select, read, convert):
    value = None
    with manager():
        for item in items:
            if select(item):
                try:
                    value = read(item)
                except ValueError:
                    pair = read(None)
                    if len(pair) == 2:
                        value = (convert(pair[0]), convert(pair[1]))
            else:
                value = item
    return value
",
        ),
        (
            "manager_handler_unconditional_tuple_copy_control.py",
            r"def consume(manager, items, select, read, convert):
    value = None
    with manager():
        for item in items:
            if select(item):
                try:
                    value = read(item)
                except ValueError:
                    pair = read(None)
                    value = (convert(pair[0]), convert(pair[1]))
            else:
                value = item
    return value
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
