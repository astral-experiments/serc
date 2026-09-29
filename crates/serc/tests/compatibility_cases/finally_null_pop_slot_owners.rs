use crate::common::assert_matches_cpython_marshal;
use std::fmt::Write as _;

#[test]
fn matches_cpython_copied_finally_reusing_consecutive_pop_block_slots() {
    for asynchronous in [false, true] {
        for returned in ["value", "work(value)"] {
            for handler in [
                "except Error:\n        raise",
                "except Error as error:\n        return error",
            ] {
                for condition in ["flag", "flag is None", "flag is not None"] {
                    let source = format!(
                        "{}def f(flag, value):\n    try:\n        {}work(value)\n        return {returned}\n    {handler}\n    finally:\n        if {condition}:\n            finish()\n",
                        if asynchronous { "async " } else { "" },
                        if asynchronous { "await " } else { "" },
                    );
                    assert_matches_cpython_marshal(&source, "consecutive_return_pop_blocks.py");
                }
            }
        }
    }
}

#[test]
fn matches_cpython_first_finally_markers_reusing_pop_block_slots() {
    for kind in ["sync", "coroutine", "generator", "async_generator"] {
        let asynchronous = matches!(kind, "coroutine" | "async_generator");
        for prefix in [
            "        result = 1 if flag else 2\n",
            "        if flag:\n            result = 1\n        else:\n            result = 2\n",
        ] {
            for condition in ["value", "value is None", "value is not None"] {
                for calls in 2..=4 {
                    let mut source = format!(
                        "{}def f(value, flag):\n",
                        if asynchronous { "async " } else { "" },
                    );
                    if matches!(kind, "generator" | "async_generator") {
                        source.push_str("    yield value\n");
                    }
                    source.push_str("    try:\n");
                    source.push_str(prefix);
                    for _ in 0..calls {
                        source.push_str("        work()\n");
                    }
                    writeln!(
                        source,
                        "    finally:\n        if {condition}:\n            {}value.close()",
                        if asynchronous { "await " } else { "" },
                    )
                    .unwrap();
                    assert_matches_cpython_marshal(
                        &source,
                        &format!("finally_null_pop_{kind}_{calls}_{condition}.py"),
                    );
                }
            }
        }
    }
}

#[test]
fn matches_cpython_finally_owners_after_constant_branch_joins() {
    for branch in [
        "if True:\n                pass",
        "if True:\n                task(value)",
    ] {
        let source = format!(
            r"def f(value, flag, task, manager):
    with manager:
        try:
            result = 1 if flag else 2
            work()
            work()
            work()
            {branch}
        finally:
            if value:
                task(value)
    after()
",
        );
        assert_matches_cpython_marshal(&source, "finally_constant_branch_join.py");
    }
}

#[test]
fn matches_cpython_nested_finally_markers_after_collection_folding() {
    for collection in [
        "[1, 2, 3]",
        "[1, 2, 3, 4]",
        "[1, 2, 3, 4, 5]",
        "{1, 2, 3}",
        "{1, 2, 3, 4}",
        "{1, 2, 3, 4, 5}",
        "(1,)",
        "(1, 2)",
        "(1, 2, 3)",
    ] {
        let source = format!(
            "def f(flag):
    try:
        try:
            consume({collection})
        finally:
            if flag:
                work()
    finally:
        finish()
"
        );
        assert_matches_cpython_marshal(&source, "nested_finally_collection_fold.py");
    }
}
