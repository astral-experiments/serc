use crate::common;

#[test]
fn matches_cpython_exception_owners_in_optimized_boolean_backedges() {
    for source in [
        r"def f(groups):
    try:
        return [group for group in groups if any(not item.disabled for item in group)]
    except Error:
        recover()
",
        r"def f(pods, prefixes):
    try:
        return [pod for pod in pods if any(pod.startswith(prefix) for prefix in prefixes)]
    except Error:
        recover()
",
        r"def f(groups):
    try:
        for group in groups:
            if not group or not any(check(item) for item in group):
                continue
            yield group
    finally:
        cleanup()
",
    ] {
        common::assert_matches_cpython_marshal(source, "optimized_boolean_backedge_owners.py");
    }
}
