use crate::common;

#[test]
fn matches_cpython_protected_terminating_branch_boolean_backedges() {
    for condition in ["flag and value is not None", "flag and ready(value)"] {
        for exit in ["return value", "continue", "break", "raise ValueError"] {
            for handler in ["pass", "continue", "observe(value)"] {
                let source = format!(
                    "def f(items, flag):\n    for value in items:\n        try:\n            observe(value)\n            if {condition}:\n                {exit}\n        except Exception:\n            {handler}\n    return None\n"
                );
                common::assert_matches_cpython_marshal(&source, "protected_tail_boolean.py");
            }
        }
    }
}
