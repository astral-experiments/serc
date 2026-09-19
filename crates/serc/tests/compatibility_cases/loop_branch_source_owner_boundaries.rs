use crate::common::assert_matches_cpython_marshal;

#[test]
fn matches_cpython_protected_loop_branch_source_owner_boundaries() {
    for kind in ["sync", "coroutine", "generator", "async_generator"] {
        for loop_header in ["for value in items", "while ready()"] {
            for condition in [
                "isinstance(value, str)",
                "check(value)",
                "predicate(value)",
                "value is None",
                "value.attr",
            ] {
                let source = format!(
                    "{}def f(value, items, predicate):\n{}    try:\n        {loop_header}:\n            if isinstance(value, int):\n                consume(value)\n            elif {condition}:\n                consume(value)\n            else:\n                fallback(value)\n    except Error:\n        recover()\n",
                    if matches!(kind, "coroutine" | "async_generator") {
                        "async "
                    } else {
                        ""
                    },
                    if matches!(kind, "generator" | "async_generator") {
                        "    yield None\n"
                    } else {
                        ""
                    },
                );
                assert_matches_cpython_marshal(
                    &source,
                    &format!("loop_branch_source_{kind}_{loop_header}_{condition}.py"),
                );
            }
        }
    }
}

#[test]
fn matches_cpython_protected_loop_branch_source_return_owners() {
    for kind in ["sync", "coroutine", "generator", "async_generator"] {
        for loop_header in ["for value in items", "while ready()"] {
            for condition in ["check(value)", "value is None"] {
                let source = format!(
                    "{}def f(value, items):\n{}    try:\n        {loop_header}:\n            if isinstance(value, int):\n                consume(value)\n            elif {condition}:\n                return\n            else:\n                fallback(value)\n    except Error:\n        recover()\n",
                    if matches!(kind, "coroutine" | "async_generator") {
                        "async "
                    } else {
                        ""
                    },
                    if matches!(kind, "generator" | "async_generator") {
                        "    yield None\n"
                    } else {
                        ""
                    },
                );
                assert_matches_cpython_marshal(
                    &source,
                    &format!("loop_branch_return_{kind}_{loop_header}_{condition}.py"),
                );
            }
        }
    }
}
