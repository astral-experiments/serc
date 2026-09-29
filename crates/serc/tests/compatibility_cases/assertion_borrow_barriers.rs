use std::fmt::Write as _;

use crate::common;
use crate::common::indent;

#[test]
fn preserves_folded_else_borrow_barriers_around_assertions() {
    for assertion in [
        "",
        "assert flag\n",
        "assert flag == other\n",
        "assert flag, value\n",
    ] {
        for placement in ["before", "after", "inside"] {
            if assertion.is_empty() && placement != "before" {
                continue;
            }
            for truthiness in ["False", "True"] {
                for has_else in [false, true] {
                    if placement == "inside" && !has_else {
                        continue;
                    }
                    let mut body = String::from("prepare(value)\n");
                    if placement == "before" {
                        body.push_str(assertion);
                    }
                    writeln!(body, "if {truthiness}:\n    unused(value)")
                        .expect("writing to a String cannot fail");
                    if has_else {
                        body.push_str("else:\n");
                        if placement == "inside" {
                            body.push_str(&indent(assertion));
                        }
                        body.push_str("    consume(value, other)\n");
                    }
                    if placement == "after" {
                        body.push_str(assertion);
                    }
                    body.push_str("return use(value, other)\n");
                    let source = format!("def function(flag, other, value):\n{}", indent(&body));
                    common::assert_matches_cpython_marshal(&source, "assertion_borrow_barrier.py");
                }
            }
        }
    }
}

#[test]
fn preserves_shared_assertion_success_borrow_boundaries() {
    for (prefix, leading) in [("", ""), ("async ", ""), ("", "yield value\n")] {
        for expression in [
            "left or right",
            "left and (right or third)",
            "not (left and right)",
            "left or ()",
            "left or (right if third else value)",
            "left and right",
        ] {
            for handler in [
                "except:",
                "except ValueError:",
                "except ValueError as error:",
            ] {
                for continuation in ["consume(result)\nreturn result\n", "return result\n"] {
                    let body = format!(
                        "{leading}try:\n    result = action(value)\n    assert {expression}\n{handler}\n    result = None\n{continuation}"
                    );
                    let source = format!(
                        "{prefix}def function(left, right, third, value):\n{}",
                        indent(&body),
                    );
                    common::assert_matches_cpython_marshal(
                        &source,
                        "shared_assertion_success_borrows.py",
                    );
                }
            }
        }
    }
}
