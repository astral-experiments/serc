use crate::common;

#[test]
fn matches_cpython_handler_fed_finally_boundaries() {
    for body in [
        "raise Error",
        "return value",
        "while True:\n                action()",
    ] {
        for finalbody in [
            "finish()",
            "result = value",
            "finish(value if value else None)\n            result = value",
        ] {
            let source = format!(
                "def f(manager, value):\n    with manager:\n        try:\n            {body}\n        except Error:\n            recover()\n        finally:\n            {finalbody}\n"
            );
            common::assert_matches_cpython_marshal(&source, "handler_fed_finally_boundary.py");
        }
    }
}
