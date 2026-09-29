use crate::common;
use std::io::Write as _;
use std::process::Stdio;

#[test]
fn matches_cpython_non_final_try_backedges_in_protected_loops() {
    for loop_header in ["for item in items:", "while items:"] {
        for handler in ["except:", "except Error as error:"] {
            for condition in ["flag", "flag and other"] {
                let source = format!(
                    "def f(items, item, flag, other, gate):\n    try:\n        {loop_header}\n            if gate:\n                try:\n                    if {condition}:\n                        consume(item)\n                {handler}\n                    recover()\n            else:\n                alternate()\n    except OuterError:\n        outer_recover()\n    finish()\n"
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "protected_non_final_try_backedges.py",
                );
            }
        }
    }
}

#[test]
fn preserves_folded_non_final_try_nop_owners() {
    for loop_header in ["for item in items:", "while items:"] {
        for handler in ["except:", "except Error as error:"] {
            let source = format!(
                "def f(items, item, flag, other, gate):\n    with resource():\n        {loop_header}\n            if gate:\n                try:\n                    if True:\n                        consume(item)\n                {handler}\n                    recover()\n            else:\n                alternate()\n    finish()\n"
            );
            common::assert_matches_cpython_marshal(&source, "folded_non_final_try_owners.py");
        }
    }
}

const NESTED_REGION_ORACLE: &str = r#"
import dis, marshal, sys, types
native_root = compile(sys.argv[1], sys.argv[2], "exec", dont_inherit=True, optimize=0)
actual_root = marshal.loads(sys.stdin.buffer.read())
native = next(c for c in native_root.co_consts if isinstance(c, types.CodeType) and c.co_name == "f")
actual = next(c for c in actual_root.co_consts if isinstance(c, types.CodeType) and c.co_name == "f")
assert actual.co_filename == native.co_filename == sys.argv[2]
ni, ai = list(dis.get_instructions(native)), list(dis.get_instructions(actual))

def backedge(instructions, offset):
    index = next(k for k, i in enumerate(instructions) if i.offset == offset)
    start = offset
    while instructions[index].opname == "EXTENDED_ARG":
        index += 1
    instruction = instructions[index]
    if instruction.opname != "JUMP_BACKWARD":
        return None
    end = instructions[index + 1].offset if index + 1 < len(instructions) else instruction.offset + 4
    return start, end, instruction

def owners(code, start, end):
    entries = dis.Bytecode(code).exception_entries
    return [next((e for e in entries if e.start <= offset < e.end), None)
            for offset in range(start, end, 2)]

def header_signature(instructions, jump):
    target = next(i for i in instructions if i.offset == jump.argval)
    return target.opname, tuple(target.positions), None if target.opcode in dis.hasjump else target.arg

# Shared backedges may still lack CPython's separate located copies. Every emitted
# conditional exit must retain the native polarity, loop header, and exception owner.
checked = 0
for expected in ni:
    if not expected.opname.startswith("POP_JUMP_IF_"):
        continue
    native_edge = backedge(ni, expected.argval)
    if native_edge is None or any(owner is not None for owner in owners(native, *native_edge[:2])):
        continue
    corresponding = [i for i in ai if i.opname.startswith("POP_JUMP_IF_") and i.positions == expected.positions]
    assert len(corresponding) == 1, ("conditional correspondence", expected, corresponding)
    observed = corresponding[0]
    assert observed.opname == expected.opname, ("conditional polarity", observed, expected)
    actual_edge = backedge(ai, observed.argval)
    assert actual_edge is not None, ("conditional backedge destination", observed, expected)
    assert header_signature(ai, actual_edge[2]) == header_signature(ni, native_edge[2]), "loop header"
    actual_owners = owners(actual, *actual_edge[:2])
    assert all(owner is None for owner in actual_owners), ("backedge owner", observed, actual_owners)
    checked += 1
assert checked > 0, "fixture must exercise a native unowned conditional backedge"
"#;

fn assert_native_conditional_backedge_owners(source: &str, filename: &str) {
    assert_native_loop_tail_component(source, filename, NESTED_REGION_ORACLE);
}

fn assert_native_loop_tail_component(source: &str, filename: &str, oracle: &str) {
    let marshal = serc::compile(source, filename).unwrap().marshal();
    let mut child = common::python_3145_command()
        .args(["-c", oracle, source, filename])
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(&marshal).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{source}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn preserves_non_final_try_backedges_crossing_nested_regions() {
    for loop_header in ["for item in items:", "while items:"] {
        for handler in ["except:", "except Error as error:"] {
            for condition in ["flag", "flag and other"] {
                let source = format!(
                    "def f(items, item, flag, other, gate):\n    {loop_header}\n        try:\n            if gate:\n                try:\n                    if {condition}:\n                        consume(item)\n                {handler}\n                    recover()\n            else:\n                alternate()\n        except OuterError:\n            outer_recover()\n    finish()\n"
                );
                assert_native_conditional_backedge_owners(
                    &source,
                    "nested_region_non_final_try_backedges.py",
                );
            }
        }
    }
}

#[test]
fn preserves_non_final_try_branch_chain_layout_and_owners() {
    for loop_header in ["for item in items:", "while items:"] {
        for handler in ["except:", "except Error as error:"] {
            for nested_region in [false, true] {
                let source = if nested_region {
                    format!(
                        "def f(items, item, flag, other, gate):\n    {loop_header}\n        try:\n            if gate:\n                try:\n                    if other:\n                        return item\n                    elif flag and item is not None:\n                        return item\n                {handler}\n                    recover()\n            elif flag:\n                return item\n        {handler}\n            recover()\n    finish()\n"
                    )
                } else {
                    format!(
                        "def f(items, item, flag, other, gate):\n    {loop_header}\n        if gate:\n            try:\n                if other:\n                    if not predicate(item):\n                        consume(item)\n                        result = item\n                elif flag:\n                    alternate()\n            {handler}\n                recover()\n        else:\n            alternate()\n    finish()\n"
                    )
                };
                assert_native_conditional_backedge_owners(
                    &source,
                    "non_final_try_branch_chains.py",
                );
            }
        }
    }
}

#[test]
fn preserves_non_final_try_nested_inert_branch_layout_and_owners() {
    for loop_header in ["for item in items:", "while items:"] {
        for handler in ["except:", "except Error as error:"] {
            for condition in ["flag", "flag and other"] {
                for body in ["pass", "field: Annotation"] {
                    let source = format!(
                        "def f(items, item, flag, other, gate):\n    {loop_header}\n        if gate:\n            try:\n                if {condition}:\n                    if other:\n                        {body}\n            {handler}\n                recover()\n        else:\n            alternate()\n    finish()\n"
                    );
                    assert_native_conditional_backedge_owners(
                        &source,
                        "non_final_try_nested_inert_branches.py",
                    );
                }
            }
        }
    }
}

const BOOLEAN_CLEANUP_ORACLE: &str = r#"
"""Allow missing redundant backedges, but require native-unlocated boolean cleanup."""
import dis
import io
import marshal
import sys
import types

filename = sys.argv[2]
native_root = compile(sys.argv[1], filename, 'exec', dont_inherit=True, optimize=0)
stream = io.BytesIO(sys.stdin.buffer.read())
actual_root = marshal.load(stream)
assert stream.tell() == len(stream.getvalue())
native = next(c for c in native_root.co_consts if isinstance(c, types.CodeType) and c.co_name == 'f')
actual = next(c for c in actual_root.co_consts if isinstance(c, types.CodeType) and c.co_name == 'f')
assert actual.co_filename == native.co_filename == filename
ni, ai = list(dis.get_instructions(native)), list(dis.get_instructions(actual))
none = (None, None, None, None)

def cleanup(code, instructions, target):
    index = next(k for k, i in enumerate(instructions) if i.offset == target)
    pop = instructions[index]
    if pop.opname != 'POP_TOP':
        return None
    end_index = index + 1
    while instructions[end_index].opname == 'EXTENDED_ARG':
        end_index += 1
    edge = instructions[end_index]
    if edge.opname != 'JUMP_BACKWARD':
        return None
    end = instructions[end_index + 1].offset if end_index + 1 < len(instructions) else len(code.co_code)
    return pop, edge, end

def header(instructions, jump):
    target = next(i for i in instructions if i.offset == jump.argval)
    return target.opname, tuple(target.positions), None if target.opcode in dis.hasjump else target.arg

checked = 0
for expected in ni:
    if not expected.opname.startswith('POP_JUMP_IF_'):
        continue
    native_cleanup = cleanup(native, ni, expected.argval)
    if native_cleanup is None:
        continue
    native_positions = list(native.co_positions())
    start, end = native_cleanup[0].offset, native_cleanup[2]
    if not all(native_positions[offset // 2] == none for offset in range(start, end, 2)):
        continue
    matching = [i for i in ai if i.opname == expected.opname and i.positions == expected.positions]
    assert len(matching) == 1, ('boolean conditional correspondence', expected, matching)
    observed = matching[0]
    actual_cleanup = cleanup(actual, ai, observed.argval)
    assert actual_cleanup is not None, ('boolean cleanup destination', observed)
    assert header(ai, actual_cleanup[1]) == header(ni, native_cleanup[1]), 'boolean loop header'
    actual_positions = list(actual.co_positions())
    start, end = actual_cleanup[0].offset, actual_cleanup[2]
    assert all(actual_positions[offset // 2] == none for offset in range(start, end, 2)), (
        'boolean cleanup must retain native NONE positions', observed,
        [actual_positions[offset // 2] for offset in range(start, end, 2)])
    checked += 1
assert checked > 0, 'fixture must exercise native-unlocated boolean cleanup'
"#;

#[test]
fn preserves_non_final_try_discarded_boolean_cleanup_locations() {
    for loop_header in ["for item in items:", "while items:"] {
        for handler in ["except:", "except Error as error:"] {
            for condition in ["flag", "flag and other"] {
                for operator in ["and", "or"] {
                    let source = format!(
                        "def f(items, item, flag, other, gate):\n    {loop_header}\n        if gate:\n            try:\n                if {condition}:\n                    consume(item) {operator} recover()\n            {handler}\n                recover()\n        else:\n            alternate()\n    finish()\n"
                    );
                    assert_native_loop_tail_component(
                        &source,
                        "non_final_try_boolean_cleanup.py",
                        BOOLEAN_CLEANUP_ORACLE,
                    );
                }
            }
        }
    }
}

const SUSPENSION_OWNER_ORACLE: &str = r#"
"""Allow missing redundant backedges, but require native-unowned suspension branch words."""
import dis
import inspect
import io
import marshal
import sys
import types

filename = sys.argv[2]
native_root = compile(sys.argv[1], filename, 'exec', dont_inherit=True, optimize=0)
stream = io.BytesIO(sys.stdin.buffer.read())
actual_root = marshal.load(stream)
assert stream.tell() == len(stream.getvalue())
native = next(c for c in native_root.co_consts if isinstance(c, types.CodeType) and c.co_name == 'f')
actual = next(c for c in actual_root.co_consts if isinstance(c, types.CodeType) and c.co_name == 'f')
assert actual.co_filename == native.co_filename == filename
assert native.co_flags & (inspect.CO_GENERATOR | inspect.CO_COROUTINE | inspect.CO_ASYNC_GENERATOR)
ni, ai = list(dis.get_instructions(native)), list(dis.get_instructions(actual))

def owner(code, offset):
    return next((e for e in dis.Bytecode(code).exception_entries if e.start <= offset < e.end), None)

checked = 0
for expected in ni:
    if expected.opname != 'NOT_TAKEN' or owner(native, expected.offset) is not None:
        continue
    matching = [i for i in ai if i.opname == expected.opname and i.positions == expected.positions]
    assert len(matching) == 1, ('suspension branch correspondence', expected, matching)
    observed = matching[0]
    assert owner(actual, observed.offset) is None, (
        'suspension NOT_TAKEN must remain native-unowned', observed, owner(actual, observed.offset))
    checked += 1
assert checked > 0, 'fixture must exercise native-unowned suspension branch words'
"#;

#[test]
fn preserves_non_final_try_suspension_branch_owners() {
    for handler in ["except:", "except Error as error:"] {
        for condition in ["flag", "flag and other"] {
            for (definition, prefix, body) in [
                ("def", "", "yield consume(item) if other else recover()"),
                ("async def", "", "await consume(item)"),
                (
                    "async def",
                    "",
                    "yield consume(item) if other else recover()",
                ),
                (
                    "def",
                    "    if False:\n        yield item\n",
                    "consume(item)",
                ),
            ] {
                let source = format!(
                    "{definition} f(items, item, flag, other, gate):\n{prefix}    while items:\n        if gate:\n            try:\n                if {condition}:\n                    {body}\n            {handler}\n                recover()\n        else:\n            alternate()\n    finish()\n"
                );
                assert_native_loop_tail_component(
                    &source,
                    "non_final_try_suspension_owners.py",
                    SUSPENSION_OWNER_ORACLE,
                );
            }
        }
    }
}

// Other fallthrough jumps still have separate location defects; this oracle checks
// every conditional-target backedge word without changing the full marshal metric.
const SHARED_NORMAL_EXIT_LOCATION_ORACLE: &str = r#"
import dis
import io
import marshal
import sys
import types

native_root = compile(sys.argv[1], sys.argv[2], "exec", dont_inherit=True, optimize=0)
stream = io.BytesIO(sys.stdin.buffer.read())
actual_root = marshal.load(stream)
assert stream.tell() == len(stream.getvalue())
native = next(c for c in native_root.co_consts if isinstance(c, types.CodeType) and c.co_name == "f")
actual = next(c for c in actual_root.co_consts if isinstance(c, types.CodeType) and c.co_name == "f")
assert actual.co_filename == native.co_filename == sys.argv[2]
assert actual.co_code == native.co_code
assert actual.co_exceptiontable == native.co_exceptiontable
instructions = list(dis.get_instructions(native, show_caches=True))
indices = {instruction.offset: index for index, instruction in enumerate(instructions)}
actual_positions = list(actual.co_positions())
native_positions = list(native.co_positions())
checked = 0
for condition in instructions:
    if not condition.opname.startswith("POP_JUMP_IF_"):
        continue
    start = condition.argval
    index = indices[start]
    while instructions[index].opname == "EXTENDED_ARG":
        index += 1
    if instructions[index].opname != "JUMP_BACKWARD":
        continue
    index += 1
    while index < len(instructions) and instructions[index].opname == "CACHE":
        index += 1
    end = instructions[index].offset if index < len(instructions) else len(native.co_code)
    assert actual_positions[start // 2:end // 2] == native_positions[start // 2:end // 2], (
        "copied conditional backedge positions", condition, start, end)
    checked += 1
assert checked > 0, "fixture must exercise a conditional backedge copy"
"#;

#[test]
fn preserves_shared_normal_finally_conditional_backedge_locations() {
    for asynchronous in [false, true] {
        for outer_action in ["return consume(result)", "consume(result)"] {
            for condition in ["result", "test(result)"] {
                let prefix = if asynchronous { "async " } else { "" };
                let action = if asynchronous {
                    "await consume(result)"
                } else {
                    "consume(result)"
                };
                let source = format!(
                    "{prefix}def f(values):\n    for result in values:\n        try:\n            {outer_action}\n        except Exception:\n            pass\n        finally:\n            if ready():\n                try:\n                    if {condition}:\n                        {action}\n                except Exception:\n                    pass\n"
                );
                assert_native_loop_tail_component(
                    &source,
                    "shared_normal_finally_backedges.py",
                    SHARED_NORMAL_EXIT_LOCATION_ORACLE,
                );
                let ordinary_join = format!("{source}                consume(result)\n");
                common::assert_matches_cpython_marshal(
                    &ordinary_join,
                    "shared_normal_finally_ordinary_join.py",
                );
            }
        }
    }
}

#[test]
fn preserves_shared_normal_try_exit_nop_removal() {
    for prefix in ["", "async "] {
        for condition in ["b and not test(c)", "b or test(c)"] {
            for terminal in ["raise Error", "return c"] {
                let source = format!(
                    "{prefix}def f(a, b, c):\n    try:\n        if a:\n            work()\n            if {condition}:\n                {terminal}\n        else:\n            work()\n            if {condition}:\n                {terminal}\n    except Error:\n        pass\n    finish()\n"
                );
                common::assert_matches_cpython_marshal(&source, "shared_normal_try_exit.py");
            }
        }
    }
}

#[test]
fn preserves_authored_exits_after_shared_normal_try_joins() {
    for try_else in [false, true] {
        for nested in [false, true] {
            for successor in ["continue", "break", "return value", "consume(value)"] {
                let protected = if try_else {
                    "try:\n    work(value)\nexcept Exception:\n    failed(value)\nelse:\n    if value:\n        work(value)"
                } else {
                    "try:\n    if value:\n        work(value)\nexcept Exception:\n    failed(value)"
                };
                let protected = format!("{protected}\n{successor}");
                let body = if nested {
                    format!(
                        "if ready():\n    {}\nafter(value)",
                        protected.replace('\n', "\n    ")
                    )
                } else {
                    protected
                };
                let source = format!(
                    "def f(values):\n    for value in values:\n        {}\n    return None\n",
                    body.replace('\n', "\n        ")
                );
                // A threaded authored continue retains its own range even when the
                // preceding normal try exit was an unlocated shared join.
                common::assert_matches_cpython_marshal(
                    &source,
                    "authored_exit_after_shared_normal_try_join.py",
                );
            }
        }
    }
}

#[test]
fn matches_cpython_nested_try_loop_tails_with_direct_continue_handlers() {
    for loop_header in ["for item in items:", "while items:"] {
        for handler in ["except Exception:", "except Exception as error:"] {
            for condition in ["other", "float(other) > 0"] {
                let source = format!(
                    "def f(items, flag, other, value):\n    {loop_header}\n        try:\n            if flag:\n                if {condition}:\n                    value += 1\n        {handler}\n            continue\n    return value\n"
                );
                common::assert_matches_cpython_marshal(&source, "nested_try_direct_continue.py");
            }
        }
    }
}

#[test]
fn matches_cpython_loop_tail_markers_in_original_call_slots() {
    for outer_try in [false, true] {
        for condition in ["isinstance(item, list)", "predicate(item)", "gate"] {
            let loop_body = format!(
                "for item in items:\n    try:\n        if gate:\n            if {condition}:\n                for child in item:\n                    consume(child)\n    except Error:\n        continue\n"
            );
            let body = if outer_try {
                format!(
                    "try:\n    {}\nexcept OuterError:\n    recover()\n",
                    loop_body.replace('\n', "\n    ").trim_end()
                )
            } else {
                loop_body
            };
            let source = format!(
                "def f(items, gate, predicate):\n    {}",
                body.replace('\n', "\n    ")
            );
            common::assert_matches_cpython_marshal(&source, "loop_tail_original_call_slots.py");
        }
    }
}

#[test]
fn matches_cpython_loop_tail_markers_in_original_none_comparison_slots() {
    for prefix in ["", "work(item)\n                "] {
        let source = format!(
            "def f(items, other):\n    for item in items:\n        try:\n            if item:\n                {prefix}if item.block.value is None or item.block.value < other.value:\n                    consume(item)\n        except Exception:\n            continue\n"
        );
        common::assert_matches_cpython_marshal(&source, "loop_tail_original_none_slots.py");
    }
}

#[test]
fn matches_cpython_loop_tail_markers_after_original_call_and_local_fusion() {
    for condition in [
        "force(control, plan=plan) or not control.input_value(timeout=2000)",
        "force(control) or not control.input_value(timeout=2000)",
        "control.ready or not control.input_value(timeout=2000)",
    ] {
        let source = format!(
            "def f(items, plan):\n    for control in items:\n        try:\n            if control:\n                if {condition}:\n                    consume(control)\n        except Exception:\n            continue\n"
        );
        common::assert_matches_cpython_marshal(&source, "loop_tail_original_fused_slots.py");
    }
}
