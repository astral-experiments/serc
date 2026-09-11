use crate::common;

#[test]
fn matches_cpython_folded_condition_at_an_empty_join() {
    for (filename, source) in [
        (
            "truthy_and_after_conditional_join.py",
            r"def scan(lines):
    found = False
    for line in lines:
        if 'first' in line:
            found = True
        if 'enabled' and 'second' in line:
            found = True
    return found
",
        ),
        (
            "false_or_after_conditional_join.py",
            r"def scan(lines):
    found = False
    for line in lines:
        if 'first' in line:
            found = True
        if 0 or 'second' in line:
            found = True
    return found
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_folded_condition_borrow_controls() {
    for (filename, source) in [
        (
            "truthy_and_in_nonempty_block.py",
            r"def scan(lines):
    found = False
    for line in lines:
        found = False
        if 'enabled' and 'second' in line:
            found = True
    return found
",
        ),
        (
            "retained_multiline_truthy_operand.py",
            r"def scan(lines):
    found = False
    for line in lines:
        if 'first' in line:
            found = True
        if (
            'enabled'
            and 'second' in line
        ):
            found = True
    return found
",
        ),
        (
            "runtime_operand_after_conditional_join.py",
            r"def scan(lines, enabled):
    found = False
    for line in lines:
        if 'first' in line:
            found = True
        if enabled and 'second' in line:
            found = True
    return found
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_taken_constant_borrow_controls() {
    for (filename, source) in [
        (
            "retained_truthy_after_nonempty_block.py",
            r"def scan(lines):
    found = False
    for line in lines:
        found = False
        if (
            'enabled'
            and 'second' in line
        ):
            found = True
    return found
",
        ),
        (
            "taken_number_after_conditional_join.py",
            r"def scan(lines):
    found = False
    for line in lines:
        if 'first' in line:
            found = True
        if 1 and 'second' in line:
            found = True
    return found
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

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
fn matches_cpython_pending_outer_boolean_entry() {
    let source = r"def f(x):
    if ready() or (True or check(x)):
        consume(x)
";
    common::assert_matches_cpython_marshal(source, "pending_outer_boolean_entry.py");
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
