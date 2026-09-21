use crate::common;

#[test]
fn conditional_handler_return_keeps_populated_recovery_cleanup() {
    for handler in ["ValueError", "ValueError as error"] {
        for prefix in [
            "",
            "            value.prepare()\n",
            "            assert value.ready is not None\n",
        ] {
            let source = format!(
                "def recover(value):\n    try:\n        value.work()\n    except {handler}:\n        if value.failed:\n{prefix}            return\n        value.recover()\n    finally:\n        value.close()\n"
            );
            common::assert_matches_cpython_marshal(&source, "handler_return_borrow.py");
        }
    }
}
