use crate::common;

#[test]
fn matches_cpython_boolean_finalizer_exit_copies() {
    for body in ["return value", "return work()", "work()", "raise Error"] {
        for expression in [
            "done() or cancel()",
            "done() and cancel()",
            "done() or cancel() or finish()",
            "done() and cancel() or finish()",
            "obj.done() or obj.cancel()",
            "first or last",
        ] {
            let source = format!(
                "def f(value, obj, first, last):\n    try:\n        {body}\n    finally:\n        {expression}\n"
            );
            common::assert_matches_cpython_marshal(&source, "boolean_finalizer_copies.py");
        }
    }
}

#[test]
fn matches_cpython_boolean_finalizer_continuation_controls() {
    for body in ["return work()", "work()"] {
        for prefix in ["", "        before()\n"] {
            for suffix in ["", "        after()\n"] {
                let source = format!(
                    "def f():\n    try:\n        {body}\n    finally:\n{prefix}        done() or cancel()\n{suffix}"
                );
                common::assert_matches_cpython_marshal(&source, "boolean_finalizer_control.py");
            }
        }
    }
}
