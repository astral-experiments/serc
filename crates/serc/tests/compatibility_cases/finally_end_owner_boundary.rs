use crate::common::assert_matches_cpython_marshal;

#[test]
fn matches_cpython_marshal_for_conditions_after_finally_end() {
    for (prefix, tail) in [
        ("def", "return value"),
        ("async def", "return value"),
        ("def", "yield value"),
        ("async def", "yield value"),
    ] {
        for condition in ["value", "value is None", "check(value)"] {
            for preparation in ["", "first, second = values"] {
                let source = format!(
                    "{prefix} f(manager, value, values):
    with manager:
        try:
            work()
        finally:
            cleanup()
        {preparation}
        if {condition}:
            act()
    {tail}
"
                );
                assert_matches_cpython_marshal(&source, "finally_end_owner_boundary.py");
            }
        }
    }
}
