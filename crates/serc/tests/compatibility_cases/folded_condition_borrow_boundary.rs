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

#[test]
fn matches_cpython_loads_after_a_discarded_if_suite() {
    for condition in ["not __debug__", "not True", "__debug__", "check()"] {
        let source = format!(
            r"def run(value, flag):
    if flag:
        if {condition}:
            other = 1
            consume(other)
        else:
            value.method()
"
        );
        common::assert_matches_cpython_marshal(&source, "discarded_if_suite.py");
    }
    common::assert_matches_cpython_marshal(
        r"def run(func, flag):
    if func is None:
        return default()
    if not __debug__ and not flag:
        return func
    if ready(func):
        consume(func)
    return func
",
        "discarded_short_circuit_suite.py",
    );
}
