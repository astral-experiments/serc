use crate::common;

#[test]
fn matches_cpython_negated_conditional_comprehension_filter() {
    let source = r"def filtered(values, predicate, selector):
    return [value for value in values if (not predicate(value) if selector else True)]
";
    common::assert_matches_cpython_marshal(source, "negated_conditional_comprehension_filter.py");
}

#[test]
fn matches_cpython_negated_conditional_loop_filter() {
    let source = r"def filtered_loop(values, predicate, selector, sink):
    for value in values:
        if not predicate(value) if selector else True:
            sink(value)
";
    common::assert_matches_cpython_marshal(source, "negated_conditional_loop_filter.py");
}

#[test]
fn matches_cpython_positive_conditional_comprehension_control() {
    let source = r"def filtered(values, predicate, selector):
    return [value for value in values if (predicate(value) if selector else True)]
";
    common::assert_matches_cpython_marshal(source, "positive_conditional_comprehension_control.py");
}

#[test]
fn matches_cpython_positive_conditional_loop_control() {
    let source = r"def filtered_loop(values, predicate, selector, sink):
    for value in values:
        if predicate(value) if selector else True:
            sink(value)
";
    common::assert_matches_cpython_marshal(source, "positive_conditional_loop_control.py");
}
