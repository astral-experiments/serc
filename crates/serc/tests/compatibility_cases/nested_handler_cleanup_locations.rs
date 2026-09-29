use crate::common;
use std::io::Write as _;
use std::process::Stdio;

#[test]
fn matches_cpython_condition_span_after_terminating_nested_handler() {
    // Only the false edge reaches the outer POP_EXCEPT when every nested path returns.
    // Runtime conjunctions keep multiple false edges; folded constants can leave one donor.
    for condition in [
        "flag",
        "flag or other",
        "flag and other",
        "flag and True",
        "True and flag",
    ] {
        for fallback in ["None", "value"] {
            let source = format!(
                "def f(flag, other, value):\n    try:\n        return primary(value)\n    except ValueError:\n        if {condition}:\n            try:\n                return secondary(value)\n            except TypeError:\n                return {fallback}\n    return None\n"
            );
            common::assert_matches_cpython_marshal(
                &source,
                "condition_span_after_terminating_nested_handler.py",
            );
        }
    }
}

#[test]
fn matches_cpython_nested_handler_cleanup_locations() {
    for binding in ["", " as error"] {
        for condition in ["flag", "flag and value", "flag or value"] {
            for nested_body in [
                "consume()",
                "result = value",
                "pass",
                "return value",
                "raise Error",
            ] {
                for successor in ["finish()", "return value"] {
                    let source = format!(
                        "def f(flag, other, value):\n    try:\n        step()\n    except Error{binding}:\n        if {condition}:\n            if other:\n                {nested_body}\n    {successor}\n"
                    );
                    common::assert_matches_cpython_marshal(
                        &source,
                        "nested_handler_cleanup_locations.py",
                    );
                }
            }
        }
    }
}

const INERT_HANDLER_ORACLE: &str = r#"
import ast, dis, marshal, sys, types
native = compile(sys.argv[1], sys.argv[2], "exec", dont_inherit=True, optimize=0)
actual = marshal.loads(sys.stdin.buffer.read())

def function(root):
    for value in root.co_consts:
        if isinstance(value, types.CodeType):
            if value.co_name == "f":
                return value
            found = function(value)
            if found is not None:
                return found

actual, native = function(actual), function(native)
if sys.argv[3] == "declaration":
    assert actual.co_code == native.co_code, "declaration changed instruction bytes"
    assert actual.co_exceptiontable == native.co_exceptiontable, "declaration changed exception bytes"
else:
    # A discarded constant currently has a separate missing-NOP discrepancy.
    # Check all emitted instruction owners, including the NOP if it is retained.
    observed = list(dis.get_instructions(actual))
    expected = list(dis.get_instructions(native))
    def signature(instruction):
        return (instruction.opname, None if instruction.opcode in dis.hasjump else instruction.arg)
    literal = next(node for node in ast.walk(ast.parse(sys.argv[1]))
                   if isinstance(node, ast.Expr) and not isinstance(node.value, ast.Call))
    span = (literal.lineno, literal.end_lineno, literal.col_offset, literal.end_col_offset)
    native_nops = [i for i in expected if i.opname == "NOP" and tuple(i.positions) == span]
    assert len(native_nops) == 1, native_nops
    if [signature(i) for i in observed] != [signature(i) for i in expected]:
        expected = [i for i in expected if i is not native_nops[0]]
    assert [signature(i) for i in observed] == [signature(i) for i in expected]
    mapping = {n.offset: a.offset for a, n in zip(observed, expected, strict=True)}
    def owner(code, offset):
        return next((e for e in dis.Bytecode(code).exception_entries
                     if e.start <= offset < e.end), None)
    assert owner(native, native_nops[0].offset) is not None
    for a, n in zip(observed, expected, strict=True):
        if a.opcode in dis.hasjump:
            assert a.argval == mapping[n.argval], (a, n)
        observed_owner, expected_owner = owner(actual, a.offset), owner(native, n.offset)
        observed_owner = None if observed_owner is None else (
            observed_owner.target, observed_owner.depth, observed_owner.lasti)
        expected_owner = None if expected_owner is None else (
            mapping[expected_owner.target], expected_owner.depth, expected_owner.lasti)
        assert observed_owner == expected_owner, (a, observed_owner, expected_owner)
"#;

#[test]
fn preserves_native_nested_inert_handler_instruction_owners() {
    for binding in ["", " as error"] {
        for (mode, body) in [
            ("declaration", "field: Annotation"),
            ("declaration", "global global_value"),
            ("declaration", "nonlocal enclosing_value"),
            ("constant", "'unused literal'"),
            ("constant", "1 + 2"),
            ("constant", "(1, 2)"),
        ] {
            let source = format!(
                "def enclosing():\n    enclosing_value = None\n    def f(flag, other):\n        try:\n            step()\n        except Error{binding}:\n            if flag:\n                if other:\n                    {body}\n        finish()\n    return f\n"
            );
            let filename = "nested_inert_handler.py";
            let marshal = serc::compile(&source, filename).unwrap().marshal();
            let mut child = common::python_3145_command()
                .args(["-c", INERT_HANDLER_ORACLE, &source, filename, mode])
                .stdin(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            child.stdin.take().unwrap().write_all(&marshal).unwrap();
            let output = child.wait_with_output().unwrap();
            assert!(
                output.status.success(),
                "{binding} / {body}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
}
