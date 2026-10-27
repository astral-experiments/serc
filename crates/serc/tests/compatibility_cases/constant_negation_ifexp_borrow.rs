use crate::common;

#[test]
fn negated_constant_selector_keeps_discarded_arm_borrow_boundary() {
    for condition in ["not True", "not 1", "not __debug__", "not 'enabled'"] {
        let source = format!(
            "def choose(value):\n    selected = value.first() if {condition} else value.second()\n    return value.after(selected)\n\ndef iterate(value):\n    yield value.first() if {condition} else value.second()\n    yield value.after()\n"
        );
        common::assert_matches_cpython_marshal(&source, "negated_selector_borrow.py");
    }
}

#[test]
fn folded_true_selector_keeps_large_continuation_borrow_boundary() {
    for condition in ["flag or True", "not (flag and False)"] {
        for selected in ["1", "value", "value()", "(value, value)"] {
            let source = format!(
                "{selected} if {condition} else fallback()\nresult = [n for n in range(10) if n or True]\nassert True or flag\nif True:\n    pass\n"
            );
            common::assert_matches_cpython_marshal(&source, "folded_true_ifexpr_borrow.py");
        }
    }
}

#[test]
fn folded_value_selector_owns_its_entry_after_a_return_guard() {
    for selected in ["value", "value.first.second", "value.selected()"] {
        for condition in ["1", "0", "gate"] {
            let source = format!(
                "def run(value, gate):\n    if gate:\n        return False\n    selected = {selected} if {condition} else value.other()\n    if selected is None:\n        return False\n    return use(selected)\n"
            );
            common::assert_matches_cpython_marshal(&source, "empty_value_selector.py");
        }
    }
}
