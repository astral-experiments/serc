use crate::common;

#[test]
fn folds_negation_after_conditional_exit_copying() {
    for value in [
        "False",
        "True",
        "255",
        "256",
        "9223372036854775808",
        "18446744073709551615",
        "18446744073709551616",
        "3.5",
        "-0.0",
        "1e1000",
        "2j",
        "1.5 + 2j",
    ] {
        for expression in [
            format!("-({value} if flag else value)"),
            format!("-(value if flag else {value})"),
            format!("-(-({value} if flag else value))"),
        ] {
            let source = format!("def f(flag, value):\n    return {expression}\n");
            common::assert_matches_cpython_marshal(&source, "copied_unary_constant.py");
        }
    }
}

#[test]
fn preserves_multiline_negation_jump_markers() {
    for value in ["255", "256", "3.5", "2j"] {
        for expression in [
            format!("-(\n        -({value} if flag else value)\n    )"),
            format!("-(\n        -(\n            -({value} if flag else value)\n        )\n    )"),
            format!("-(\n        -(value if flag else {value})\n    )"),
        ] {
            let source = format!("def f(flag, value):\n    return {expression}\n");
            common::assert_matches_cpython_marshal(&source, "multiline_copied_negation.py");
        }
    }
}

#[test]
fn retains_operand_construction_before_folded_negation() {
    for operand in ["1.5 + 2.0", "1.5 + 2j", "-255", "-256"] {
        let source = format!(
            "def f(flag, value, other):\n    if other:\n        return -({operand} if flag else value)\n    return {operand}\n"
        );
        common::assert_matches_cpython_marshal(&source, "unary_operand_construction.py");
    }
}

#[test]
fn preserves_original_markers_through_nested_tuple_folding() {
    for value in ["255", "256", "3.5", "2j"] {
        for expression in [
            format!("(\n        -({value} if flag else x), 2\n    )"),
            format!("(\n        -(\n            {value} if flag else x\n        ), 2\n    )"),
            format!("(\n        (-({value} if flag else x),),\n    )"),
            format!(
                "(\n        (\n            -(\n                {value} if flag else x\n            ),\n        ),\n    )"
            ),
        ] {
            let source = format!("def f(flag, x):\n    return {expression}\n");
            common::assert_matches_cpython_marshal(&source, "nested_tuple_negation.py");
        }
    }
}

#[test]
fn folds_boolean_negation_in_each_scope() {
    for value in ["False", "True"] {
        for source in [
            format!("value = -{value}\n"),
            format!("class C:\n    value = -{value}\n"),
            format!("def f():\n    yield -{value}\n"),
        ] {
            common::assert_matches_cpython_marshal(&source, "boolean_negation.py");
        }
    }
}

#[test]
fn keeps_unsupported_negation_for_runtime() {
    for value in ["None", "'text'", "b'text'", "(1, 2)", "..."] {
        let source = format!("def f(flag, value):\n    return -({value} if flag else value)\n");
        common::assert_matches_cpython_marshal(&source, "runtime_negation.py");
    }
}
