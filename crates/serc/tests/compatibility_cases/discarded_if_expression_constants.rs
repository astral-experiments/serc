use crate::common;

#[test]
fn matches_cpython_discarded_if_expression_constant_admission() {
    for (discarded, reused) in [
        ("0.8", "0.8"),
        ("consume(0.8, 1.4)", "0.8"),
        ("consume('reused word')", "'reused word'"),
        ("consume(())", "()"),
        ("consume(300 + 500)", "800"),
        ("consume(-1.5)", "-1.5"),
        ("None", "None"),
    ] {
        for truth in [false, true] {
            let conditional = if truth {
                format!("value if True else {discarded}")
            } else {
                format!("{discarded} if False else value")
            };
            for body in [
                format!(
                    "    result = {conditional}\n    consume(-1)\n    return result, {reused}\n"
                ),
                format!(
                    "    if flag:\n        return {conditional}\n    consume(-1)\n    return {reused}\n"
                ),
                format!(
                    "    try:\n        result = {conditional}\n    except Error:\n        consume('failed')\n    consume(-1)\n    return result, {reused}\n"
                ),
            ] {
                let source = format!("def f(value, flag):\n    consume('before')\n{body}");
                common::assert_matches_cpython_marshal(
                    &source,
                    "discarded_if_expression_constants.py",
                );
            }
        }
    }
}

#[test]
fn matches_cpython_discarded_if_expression_folds_through_copied_exits() {
    for selected in ["3.5", "(1, 2)"] {
        for discarded in ["consume(-1.5)", "consume(())"] {
            for truth in [false, true] {
                let conditional = if truth {
                    format!("{selected} if True else {discarded}")
                } else {
                    format!("{discarded} if False else {selected}")
                };
                for expression in [format!("-({conditional})"), format!("({conditional},)")] {
                    for body in [
                        format!("    result = {expression}\n    consume(-1)\n    return result\n"),
                        format!(
                            "    if flag:\n        return {expression}\n    consume(-1)\n    return ()\n"
                        ),
                    ] {
                        let source = format!("def f(value, flag):\n    consume('before')\n{body}");
                        common::assert_matches_cpython_marshal(
                            &source,
                            "discarded_if_expression_copied_exits.py",
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn matches_cpython_nested_discarded_if_expression_exit_locations() {
    for header in ["", "def f():\n"] {
        for expression in [
            "f1 if c1 else f2 if c2 else f3 if c3 else None",
            "(f1 if c1 else f2) if c2 else f3",
            "(f1 if c1 else f2) if c2 else (f3 if c3 else None)",
            "f1() if c1 else f2() if c2 else None",
            "[x for x in items] if c1 else f2 if c2 else None",
            "f1 if c1 else f2",
        ] {
            let indentation = if header.is_empty() { "" } else { "    " };
            let source = format!("{header}{indentation}{expression}\n");
            common::assert_matches_cpython_marshal(&source, "nested_discarded_if.py");
        }
    }
}
