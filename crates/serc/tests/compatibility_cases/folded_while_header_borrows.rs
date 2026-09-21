use std::fmt::Write as _;

use crate::common;
use crate::common::indent;

#[test]
fn preserves_empty_while_headers_after_leading_boolean_folds() {
    for (declaration, is_async, is_generator) in [
        ("def", false, false),
        ("async def", true, false),
        ("def", false, true),
        ("async def", true, true),
    ] {
        for condition in [
            "True and flag",
            "False or flag",
            "__debug__ and flag",
            "flag",
            "(left if False else flag) and other",
        ] {
            for preceding_work in [false, true] {
                let mut body = String::new();
                if is_generator {
                    body.push_str("yield value\n");
                }
                if is_async {
                    body.push_str("await ready()\n");
                }
                if preceding_work {
                    body.push_str("prepare(value)\n");
                }
                writeln!(body, "while {condition}:").expect("writing to a String cannot fail");
                body.push_str(
                    "    consume(value)\n    if stop(value):\n        break\n    flag = advance(flag)\nconsume(value, other)\n",
                );
                body.push_str(if is_async && is_generator {
                    "yield value\nreturn\n"
                } else {
                    "return value\n"
                });
                let source = format!(
                    "{declaration} function(flag, other, left, value, manager, values):\n{}",
                    indent(&body),
                );
                common::assert_matches_cpython_marshal(&source, "folded_while_header.py");
            }
        }
    }
}

#[test]
fn preserves_borrowing_through_located_while_header_nops() {
    for condition in [
        "True\nand flag",
        "False\nor flag",
        "(not\n False)\nand flag",
        "(True\n and flag)\nand other",
    ] {
        for preceding_work in [false, true] {
            for with_comment in [false, true] {
                let mut body = String::new();
                if preceding_work {
                    body.push_str("prepare(value)\n");
                }
                let condition = if with_comment {
                    condition.replace('\n', "\n\n# Keep a separate line for the next operand.\n")
                } else {
                    condition.to_owned()
                };
                body.push_str("while (\n");
                body.push_str(&indent(&condition));
                body.push_str("):\n");
                body.push_str(
                    "    consume(value)\n    if stop(value):\n        break\n    flag = advance(flag)\nconsume(value, other)\nreturn value\n",
                );
                let source = format!("def function(flag, other, value):\n{}", indent(&body));
                common::assert_matches_cpython_marshal(&source, "located_while_header.py");
            }
        }
    }
}
