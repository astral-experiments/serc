use crate::common;

#[test]
fn terminal_while_admits_generator_code_before_later_literals() {
    for expression in [
        "all(value for value in values)",
        "any(value for value in values)",
        "consume(value for value in values)",
        "(value for value in values)",
        "all(value + 2 for value in values)",
        "all(value for value in (1, 2))",
        "all(value for value in get_values(2))",
        "all([value for value in values])",
        "all((lambda: values)())",
    ] {
        for tail in ["x += 1", "x += 257", "pass", "consume(1)"] {
            let source = format!(
                "def f(values):\n    while values:\n        if {expression}:\n            break\n        {tail}\n"
            );
            common::assert_matches_cpython_marshal(
                &source,
                "generator_expression_first_constant.py",
            );
        }
    }
}
