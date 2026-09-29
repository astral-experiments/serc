use crate::common;

#[test]
fn matches_cpython_nested_handler_fallthrough_cleanup() {
    for capture in ["", " as error"] {
        let source = format!(
            "def f(value):\n    try:\n        work(value)\n    except Error{capture}:\n        try:\n            return value\n        except Error:\n            pass\n    return value\n"
        );
        common::assert_matches_cpython_marshal(&source, "nested_handler_fallthrough_cleanup.py");
    }
}
