use crate::common;

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
