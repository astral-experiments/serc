use crate::common;

#[test]
fn matches_cpython_retained_pop_fused_prefix_owner() {
    for count in [1, 4, 9] {
        let prefix = "        sink(left, right)\n".repeat(count);
        let source = format!(
            "def f(sink, left, right, options):\n    try:\n{prefix}        return 0\n    finally:\n        if not options.keep:\n            sink(left, right)\n"
        );
        common::assert_matches_cpython_marshal(&source, "retained_pop_fused_prefix.py");
    }
}

#[test]
fn matches_cpython_retained_pop_join_and_unfused_controls() {
    for prefix in [
        "        sink()\n",
        "        if left:\n            sink(left, right)\n",
        "        sink(left, right)\n        if left:\n            return 1\n",
    ] {
        let source = format!(
            "def f(sink, left, right, options):\n    try:\n{prefix}        return 0\n    finally:\n        if not options.keep:\n            sink(left, right)\n"
        );
        common::assert_matches_cpython_marshal(&source, "retained_pop_prefix_controls.py");
    }
}
