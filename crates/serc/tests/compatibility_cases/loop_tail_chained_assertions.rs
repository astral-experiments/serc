use crate::common;

#[test]
fn matches_cpython_loop_tail_chained_assertions() {
    for header in ["for value in items:", "while ready():"] {
        for condition in [
            "0 <= value < 4",
            "lower() < value < upper()",
            "left is value is None",
        ] {
            for message in ["", ", 'invalid'", ", format_message(value)"] {
                let source = format!(
                    "def f(items, value, left):\n    {header}\n        assert {condition}{message}\n"
                );
                common::assert_matches_cpython_marshal(&source, "loop_tail_chained_assertions.py");
            }
        }
    }
}

#[test]
fn matches_cpython_non_final_branch_chained_assertions() {
    for header in ["for value in items:", "while ready():"] {
        let source = format!(
            "def f(items, value, flag):\n    {header}\n        if flag:\n            assert 0 <= value < 4\n        else:\n            consume(value)\n"
        );
        common::assert_matches_cpython_marshal(&source, "branch_chained_assertions.py");
    }
}

#[test]
fn matches_cpython_multiline_loop_tail_chained_assertions() {
    for header in ["for value in items:", "while ready():"] {
        for condition in ["left < value < right", "lower() < value.attr < upper()"] {
            for message in ["", ", 'invalid'", ", format_message(value)"] {
                let source = format!(
                    "def f(items, left, right):\n    {header}\n        assert (\n            {condition}\n        ){message}\n"
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "multiline_loop_tail_chained_assertions.py",
                );
            }
        }
    }
}
