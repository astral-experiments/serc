use crate::common;

#[test]
fn matches_cpython_late_constant_boolean_folding() {
    for expression in [
        "... or 2",
        "not ()",
        "not (1, 2)",
        "not (call(),)",
        "(1, 2) or (-1, 1)",
        "() and (1, 2)",
        "() or (1, 2)",
        "(1, 2) and ()",
        "(1 + 2) or 3",
        "(-1) or 2",
        "(not __debug__) and 'on' or ''",
        "(not True) or 2",
        "(not False) and 2",
        "False or not (1, 2)",
        "True and not ()",
        "True or not (call(),)",
        "False and not (call(),)",
    ] {
        for template in ["result = EXPR\n", "def f():\n    return EXPR\n"] {
            let source = template.replace("EXPR", expression);
            common::assert_matches_cpython_marshal(&source, "late_constant_boolean.py");
        }
    }
}

#[test]
fn matches_cpython_discarded_tuple_negation_folding() {
    common::assert_matches_cpython_marshal(
        "seed = 0\nresult = ... and 2\n",
        "early_ellipsis_boolean.py",
    );
    for expression in ["not ()", "not (1, 2)", "not (call(),)"] {
        let source = format!("def f(items):\n    for item in items:\n        {expression}\n");
        common::assert_matches_cpython_marshal(&source, "discarded_tuple_negation.py");
    }
}

#[test]
fn folded_nested_value_tests_keep_the_discarded_operand_boundary() {
    for expression in [
        "False or (True and key)",
        "False or\n                    (True and key)",
        "False or (True and\n                           key)",
        "False or (other and key)",
        "other or (True and key)",
    ] {
        let source = format!(
            "def choose(item, key, other):\n    before()\n    if item.tag == ({expression}):\n        use(item)\n    return item\n"
        );
        common::assert_matches_cpython_marshal(&source, "nested_folded_values.py");
    }
}
