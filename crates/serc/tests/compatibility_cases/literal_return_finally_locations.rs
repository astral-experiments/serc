use std::fmt::Write;

use crate::common;

#[test]
fn matches_cpython_literal_returns_after_finally_cleanup() {
    for prefix in ["", "async "] {
        for value in ["False", "None", "1"] {
            for cleanup_count in [1, 2] {
                let mut cleanup = String::new();
                for index in 0..cleanup_count {
                    writeln!(cleanup, "        cleanup({index})").unwrap();
                }
                let source = format!(
                    "{prefix}def f(value):\n    try:\n        work()\n        return value\n    except ValueError:\n        return {value}\n    finally:\n{cleanup}"
                );
                common::assert_matches_cpython_marshal(&source, "literal_return_finally.py");
            }
        }
    }
}

#[test]
fn matches_cpython_shared_returns_after_conditional_finally_cleanup() {
    for prefix in ["", "async "] {
        for value in ["value", "work(value)"] {
            for condition in ["flag", "flag is not None"] {
                let source = format!(
                    "{prefix}def f(value, flag):\n    try:\n        return {value}\n    finally:\n        if {condition}:\n            cleanup(value)\n        if {condition}:\n            cleanup(value)\n"
                );
                common::assert_matches_cpython_marshal(&source, "conditional_return_finally.py");
            }
        }
    }
}

#[test]
fn matches_cpython_return_expression_joins_before_finally() {
    for prefix in ["", "async "] {
        for value in [
            "value if flag else other",
            "tuple(item for item in value)",
            "value and other",
            "consume(value)",
        ] {
            for cleanup in ["cleanup()", "if flag:\n            cleanup()"] {
                let source = format!(
                    "{prefix}def f(value, other, flag):\n    try:\n        return {value}\n    finally:\n        {cleanup}\n"
                );
                common::assert_matches_cpython_marshal(&source, "return_join_finally.py");
            }
        }
    }
}

#[test]
fn matches_cpython_split_return_lines_before_finally() {
    for prefix in ["", "async "] {
        for value in ["value", "consume(value)", "1"] {
            let source = format!(
                "{prefix}def f(value):\n    try:\n        return (\n            {value}\n        )\n    finally:\n        cleanup()\n"
            );
            common::assert_matches_cpython_marshal(&source, "split_return_finally.py");
        }
    }
}
