use crate::common;

#[test]
fn matches_cpython_short_circuit_constant_borrow_entries() {
    for (filename, condition) in [
        ("false_and_dead_operand.py", "0 and check(x)"),
        ("true_or_dead_operand.py", "True or check(x)"),
        ("nested_dead_operand.py", "(0 and check(x)) or ready()"),
        ("independent_false_entry.py", "ready() and 0 and check(x)"),
    ] {
        let source = format!(
            r"def f(x, y):
    prepare()
    if {condition}:
        if other():
            return
        dead(x)
    else:
        consume(x)
    z = make(x, y)
    finish(z, y)
"
        );
        common::assert_matches_cpython_marshal(&source, filename);
    }
}

#[test]
fn matches_cpython_folded_elif_with_pending_chain_join() {
    for (filename, condition) in [
        ("false_and_elif.py", "False and (mode == 48 or mode == 64)"),
        ("zero_and_elif.py", "0 and (mode == 48 or mode == 64)"),
        ("true_or_elif.py", "True or (mode == 48 or mode == 64)"),
        ("false_elif_control.py", "False"),
        (
            "runtime_elif_control.py",
            "ready() and (mode == 48 or mode == 64)",
        ),
    ] {
        let source = format!(
            r"def f(mode, value):
    prepare()
    if mode == 32:
        consume(value)
    elif mode <= 16:
        consume(value)
    elif {condition}:
        dead(value)
    elif mode == 48 or mode == 64:
        consume(value)
    else:
        consume(value)
    finish(value)
"
        );
        common::assert_matches_cpython_marshal(&source, filename);
    }
}
