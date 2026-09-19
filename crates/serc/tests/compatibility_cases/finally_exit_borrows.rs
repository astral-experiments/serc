#[test]
fn matches_cpython_retained_nops_after_finally_borrow_barriers() {
    for prefix in ["", "async "] {
        let source = format!(
            "{prefix}def f(values, manager):\n    with manager:\n        try:\n            yield values\n        finally:\n            for value in values:\n                with manager:\n                    consume(value)\n"
        );
        common::assert_matches_cpython_marshal(&source, "finally_borrow_barrier_nop.py");
    }
}

use crate::common;

#[test]
fn matches_cpython_borrows_after_finally_loop_joins() {
    for prefix in ["", "async "] {
        for loop_body in [
            "with manager:\n    consume(item)",
            "if flag:\n    with manager:\n        consume(item)",
            "if flag:\n    consume(item)",
        ] {
            for later in ["", "\nfor other in values:\n    consume(other)"] {
                let finalbody = format!(
                    "for item in values:\n    {}{later}",
                    loop_body.replace('\n', "\n    "),
                );
                for successor in [
                    "return value",
                    "return value, other, flag, values, manager",
                    "consume(value)\nconsume(other)\nreturn value",
                ] {
                    let source = format!(
                        "{prefix}def guarded(value, other, flag, values, manager):\n    try:\n        consume(value)\n    finally:\n        {}\n    {}\n",
                        finalbody.replace('\n', "\n        "),
                        successor.replace('\n', "\n    "),
                    );
                    common::assert_matches_cpython_marshal(&source, "finally_loop_joins.py");
                }
            }
        }
    }
}

#[test]
fn matches_cpython_borrows_after_structured_finally_exits() {
    for prefix in ["", "async "] {
        for finalbody in [
            "with manager:\n    consume(value)",
            "for item in values:\n    if flag:\n        consume(item)",
            "for item in values:\n    consume(item)",
            "try:\n    consume(value)\nexcept Error:\n    pass",
            "try:\n    consume(value)\nfinally:\n    consume(other)",
        ] {
            for successor in [
                "return value",
                "return value, other, flag, values, manager",
                "consume(value)\nconsume(other)\nreturn value",
            ] {
                let body = format!(
                    "try:\n    consume(value)\nfinally:\n    {}\n{successor}",
                    finalbody.replace('\n', "\n    "),
                );
                for outer in ["", "if flag:", "while other:"] {
                    let body = if outer.is_empty() {
                        body.clone()
                    } else {
                        format!(
                            "{outer}\n    {}\nreturn other",
                            body.replace('\n', "\n    "),
                        )
                    };
                    let source = format!(
                        "{prefix}def guarded(value, other, flag, values, manager):\n    {}\n",
                        body.replace('\n', "\n    "),
                    );
                    common::assert_matches_cpython_marshal(&source, "finally_exit_borrows.py");
                }
            }
        }
    }
}
