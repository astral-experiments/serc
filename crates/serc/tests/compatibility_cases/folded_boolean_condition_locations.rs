use crate::common;

#[test]
fn preserves_folded_boolean_operand_constant_order() {
    for expression in [
        "None or 'second'",
        "False or 'second'",
        "True and 'second'",
        "'' or 'second'",
        "256 and 'second'",
        "None or False or 'second'",
    ] {
        for prefix in ["", "    version = 'first'\n"] {
            for suffix in [
                "    yield value\n",
                "    return None, False, True, '', 256, value\n",
                "    return value\n",
            ] {
                let source = format!("def f():\n{prefix}    value = {expression}\n{suffix}");
                common::assert_matches_cpython_marshal(&source, "folded_boolean_constants.py");
            }
        }
    }
}

#[test]
fn intermediate_folded_boolean_operand_keeps_its_empty_borrow_block() {
    for value in [
        "None or 'value'",
        "None or False or 'value'",
        "None or 'value' or False",
        "True and True and 'value'",
        "False or None or False or 'value'",
    ] {
        let source = format!("def f():\n    value = {value}\n    yield value\n");
        common::assert_matches_cpython_marshal(&source, "folded_boolean_borrow_block.py");
    }
}

#[test]
fn preserves_folded_boolean_condition_locations() {
    for condition in [
        "True and False and 1 and 2",
        "0 or 0",
        "1 and 1",
        "False or True or 0",
        "True and (False or False)",
        "True and (True and False)",
        "flag and True",
        "True and flag",
        "flag or False",
        "False and flag",
    ] {
        for body in ["pass", "work()", "\n    pass"] {
            for otherwise in ["", "else: recover()\n"] {
                let source = format!("if {condition}: {body}\n{otherwise}finish()\n");
                common::assert_matches_cpython_marshal(
                    &source,
                    "folded_boolean_condition_locations.py",
                );
            }
        }
    }
}

#[test]
fn preserves_folded_boolean_locations_across_lines_and_runtime_operands() {
    for condition in [
        "True and False",
        "True and\n        False",
        "0 or\n        0",
        "1 and\n        1",
        "flag and True and False",
        "flag or False or True",
    ] {
        for body in ["pass", "work()"] {
            let source = format!(
                "def f(flag):\n    if ({condition}): {body}\n    else: recover()\n    finish()\n"
            );
            common::assert_matches_cpython_marshal(
                &source,
                "folded_boolean_multiline_locations.py",
            );
        }
    }
}

#[test]
fn preserves_folded_boolean_loop_exit_locations() {
    for condition in ["1 and 1", "flag and True", "True or flag"] {
        for prologue in ["", "        prepare()\n"] {
            let source = format!(
                "def f(flag):\n    try:\n{prologue}        while {condition}: break\n    finally:\n        finish()\n"
            );
            common::assert_matches_cpython_marshal(&source, "folded_boolean_loop_locations.py");
        }
    }
}

#[test]
fn keeps_folded_assertion_failure_after_nested_boolean_tests() {
    for condition in [
        "(a or False) and (b or True)",
        "(a or False) and True",
        "((a or False) and (b or False)) or c",
        "not ((a and True) or (b and False))",
    ] {
        let source = format!("assert {condition}\nresult = a\n");
        common::assert_matches_cpython_marshal(&source, "nested_folded_assertion_failure.py");
    }
}

#[test]
fn pending_call_inputs_cross_the_folded_operand_boundary() {
    for argument in ["None or 0", "True and 1", "True or 0"] {
        let source = format!("def f(value):\n    return consume(value, {argument})\n");
        common::assert_matches_cpython_marshal(&source, "folded_operand_stack.py");
    }
}
