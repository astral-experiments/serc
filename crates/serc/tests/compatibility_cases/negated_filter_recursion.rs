use crate::common;

#[test]
fn matches_cpython_recursive_negated_comprehension_filters() {
    for expression in [
        "[x for x in items if CONDITION]",
        "{x for x in items if CONDITION}",
        "{x: transform(x) for x in items if CONDITION}",
        "(x for x in items if CONDITION)",
    ] {
        for condition in [
            "not not (first(x) or second(x))",
            "not not (first(x) and second(x))",
            "not not not (first(x) or second(x))",
            "not (first(x) or not second(x))",
            "not ((first(x) and second(x)) or third(x))",
            "not (first(x) or (second(x) and third(x)))",
            "not (first(x) and not (second(x) or third(x)))",
            "not (first(x) and second(x) and not third(x))",
            "not (first(x) or (second(x) and not third(x)))",
        ] {
            let expression = expression.replace("CONDITION", condition);
            let source = format!(
                "def collect(items, others, first, second, third, fourth, flag, limit, manager, keep, transform):\n    return {expression}\n"
            );
            common::assert_matches_cpython_marshal(&source, "negated_filter_recursion.py");
        }
    }
}
