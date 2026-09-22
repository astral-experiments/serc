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
