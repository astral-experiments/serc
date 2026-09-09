use crate::common;

#[test]
fn returns_unwind_try_markers_before_manager_cleanup() {
    for (prefix, manager) in [("", "with"), ("async ", "with"), ("async ", "async with")] {
        for value in ["", "True", "None", "3", "value", "work()"] {
            for binding in ["", " as error"] {
                let source = format!(
                    "{prefix}def f(value):\n    {manager} manager():\n        try:\n            consume()\n            return {value}\n        except Error{binding}:\n            recover()\n            return False\n"
                );
                common::assert_matches_cpython_marshal(&source, "return_try_manager_nop.py");
            }
        }
    }
}
