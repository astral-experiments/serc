use std::fmt::Write;

use crate::common;
use crate::common::indent;

#[test]
fn preserves_numeric_dependencies_while_ordering_copied_return_tuples() {
    for expression in [
        "-1",
        "-300",
        "-(1 << 63)",
        "(300 + 1) * 2",
        "('left',) + ('right',)",
        "((1, 2), (3, 4))[0]",
    ] {
        for position in ["before", "inside", "handler"] {
            for definition in ["def", "async def"] {
                for loop_body in [false, true] {
                    let mut body = String::new();
                    if position == "before" {
                        writeln!(body, "consume({expression})").unwrap();
                    }
                    body.push_str("try:\n");
                    if position == "inside" {
                        writeln!(body, "    consume({expression})").unwrap();
                    }
                    if loop_body {
                        body.push_str("    for value in values:\n        call(value)\n");
                    } else {
                        body.push_str("    call()\n");
                    }
                    body.push_str("except Error:\n");
                    if position == "handler" {
                        writeln!(body, "    consume({expression})").unwrap();
                    }
                    body.push_str("    return True, False\nreturn True, True\n");
                    common::assert_matches_cpython_marshal(
                        &format!("{definition} f(values):\n{}", indent(&body)),
                        "copied_return_constant_dependencies.py",
                    );
                }
            }
        }
    }
}

#[test]
fn preserves_reused_folds_from_conditionally_discarded_branches() {
    for expression in [
        "2048 + 1",
        "-(1 << 63)",
        "(300 + 1) * 2",
        "600 // 2",
        "'x' * 4",
        "('x',) * 3",
        "(1, 2) + (3,)",
        "(1, 2, 3)[1:]",
    ] {
        for header in [
            "if False:\nBODY",
            "if True:\n    pass\nelse:\nBODY",
            "while False:\nBODY",
            "while True:\n    break\nelse:\nBODY",
        ] {
            for definition in ["module", "def", "async def"] {
                for unreachable_tail in [false, true] {
                    let prefix = if unreachable_tail {
                        "    raise Error\n"
                    } else {
                        ""
                    };
                    let discarded = format!("{prefix}    consume({expression})\n");
                    let body = format!(
                        "consume((7, 8))\n{}result = [17, 19, 23]\nconsume({expression})\n",
                        header.replace("BODY", &discarded)
                    );
                    let source = if definition == "module" {
                        body
                    } else {
                        format!("{definition} f():\n{}", indent(&body))
                    };
                    common::assert_matches_cpython_marshal(
                        &source,
                        "discarded_branch_constant_folds.py",
                    );
                }
            }
        }
    }
}

#[test]
fn preserves_first_fold_origin_for_equal_cached_and_small_integer_results() {
    for (expression, cached) in [
        ("(260 - 5) + 0", "255"),
        ("+(260 - 5)", "255"),
        ("((260 - 5) + 0) + 0", "260 - 5"),
        ("(260 - 5) - (260 - 5)", "0"),
        ("('ab' + '')[:2]", "'ab'"),
        ("((1, 2) + ())[:]", "(1, 2)"),
        ("((1, 2),)[0][:]", "(1, 2)"),
        ("(-0.0, 0.0)[:]", "(-0.0, 0.0)"),
    ] {
        for template in [
            "consume(EXPR)\n",
            "if False:\n    if value:\n        raise Error\n        consume(EXPR)\n    else:\n        consume(EXPR)\n",
            "try:\n    consume(EXPR)\nexcept Error:\n    return True, False\n",
        ] {
            for precached in [false, true] {
                let prefix = if precached {
                    format!("consume({cached})\n")
                } else {
                    String::new()
                };
                let body = format!(
                    "{prefix}{}consume((17, 19, 23))\nconsume({expression})\nconsume({cached})\nreturn True, True\n",
                    template.replace("EXPR", expression)
                );
                common::assert_matches_cpython_marshal(
                    &format!("def f(value):\n{}", indent(&body)),
                    "equal_folded_constant_origins.py",
                );
            }
        }
    }
}
