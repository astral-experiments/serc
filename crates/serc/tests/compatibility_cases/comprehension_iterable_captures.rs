use crate::common;

#[test]
fn captures_comprehension_targets_from_nested_generator_first_iterables() {
    for element in [
        "consume(value for value in (item for item in values if item != index))",
        "consume(value for value in transform(item for item in values if item != index))",
        "consume(value for value in (item for item in (part for part in values if part != index)))",
        "consume((item for item in values if item != index) for value in values)",
        "consume(value for value in values[index])",
        "consume(value for value in (lambda: index)())",
        "consume(value for value in (lambda action=(item for item in values if item != index): action)())",
    ] {
        for (open, close, key) in [("[", "]", ""), ("{", "}", ""), ("{", "}", "index: ")] {
            let source = format!(
                "def function(values, indices):\n    return {open}{key}{element} for index in indices{close}\n"
            );
            common::assert_matches_cpython_marshal(&source, "comprehension_iterable_capture.py");
        }
    }
}
