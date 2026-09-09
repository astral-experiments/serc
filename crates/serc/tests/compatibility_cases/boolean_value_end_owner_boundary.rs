use crate::common::assert_matches_cpython_marshal;

#[test]
fn matches_cpython_marshal_for_boolean_value_end_owner_boundaries() {
    for boolean in [
        "flag or 0",
        "flag and 0",
        "(flag or other) and flag",
        "(flag and other) or flag",
    ] {
        for condition in ["flag", "check(flag)"] {
            let source = format!(
                r"def f(values, flag, other):
    for value in values:
        result = {boolean}
        try:
            result = 1 if {condition} else 2
        except Error:
            pass
"
            );
            assert_matches_cpython_marshal(&source, "boolean_value_end_owner_boundary.py");
        }
    }
}
