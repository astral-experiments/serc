use crate::common;

#[test]
fn matches_cpython_marshal_for_sync_manager_original_false_edge() {
    let source = r"def f(manager):
    try:
        with manager:
            while True:
                step()
    finally:
        finish()
";
    common::assert_matches_cpython_marshal(source, "finally_sync_with_true_loop.py");
    let source = r"def f(manager, running):
    try:
        with manager:
            while running():
                step()
    finally:
        finish()
";
    common::assert_matches_cpython_marshal(source, "finally_sync_with_runtime_loop_control.py");
}
