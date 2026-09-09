use crate::common;

#[test]
fn matches_cpython_removed_assertion_borrow_boundaries() {
    for (scope, prefix) in [
        ("", ""),
        ("", "    yield value\n"),
        ("async ", "    await ready()\n"),
    ] {
        for condition in ["True", "f'ready'"] {
            for otherwise in ["", "    else:\n        work()\n"] {
                let source = format!(
                    "{scope}def collect(value, flag):\n{prefix}    if flag:\n        assert {condition}\n{otherwise}    return value\n"
                );
                common::assert_matches_cpython_marshal(&source, "removed_assertion_boundary.py");
            }
        }
    }
}

#[test]
fn matches_cpython_threaded_assertion_borrow_boundaries() {
    for condition in ["True", "f'ready'"] {
        for otherwise in ["return value", "raise Error", "assert True"] {
            let source = format!(
                "def collect(value, flag):\n    if flag:\n        assert {condition}\n    else:\n        {otherwise}\n    consume(value)\n    return value\n"
            );
            common::assert_matches_cpython_marshal(&source, "threaded_assertion_boundary.py");
        }
    }
}
