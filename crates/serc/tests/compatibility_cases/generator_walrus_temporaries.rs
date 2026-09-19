use crate::common;

#[test]
fn keeps_generator_body_comprehension_temporaries_out_of_enclosing_locals() {
    for value in [
        "[element for element in values]",
        "{element for element in values}",
        "{element: element for element in values}",
        "[[inner for inner in element] for element in values]",
        "(lambda item=[element for element in values]: item)",
    ] {
        for (before, after) in [
            ("", ""),
            ("    element = 1\n", ""),
            ("", "    element = 1\n"),
        ] {
            for expression in [
                format!("((matched := {value}) for item in values)"),
                format!("(item for item in values if (matched := {value}))"),
            ] {
                let source = format!(
                    "def function(values):\n{before}    result = consume({expression})\n{after}    return result\n"
                );
                common::assert_matches_cpython_marshal(&source, "generator_walrus_temporaries.py");
            }
        }
    }
}

#[test]
fn keeps_nested_generator_comprehension_captures_in_their_own_scope() {
    for expression in [
        "((matched := item) for item in [element for element in values])",
        "((matched := item) for item in (nested for nested in [element for element in values]))",
        "(((matched := nested) for nested in [element for element in values]) for item in values)",
        "(consume((matched := [(lambda: element) for element in values]) for item in values) for outer_item in values)",
        "(consume((lambda arg=[lambda: element for element in values]: arg)()) for item in values)",
        "(consume((lambda arg={lambda: element for element in values}: arg)()) for item in values)",
        "(consume((lambda arg={element: (lambda: element) for element in values}: arg)()) for item in values)",
        "(consume(nested for nested in [lambda: element for element in values]) for item in values)",
        "(consume(nested for nested in {lambda: element for element in values}) for item in values)",
        "(consume(nested for nested in {element: (lambda: element) for element in values}) for item in values)",
    ] {
        for (before, after) in [
            ("", ""),
            ("    element = 1\n", ""),
            ("", "    element = 1\n"),
        ] {
            let source = format!(
                "def function(values):\n{before}    result = consume({expression})\n{after}    return result\n"
            );
            common::assert_matches_cpython_marshal(
                &source,
                "generator_comprehension_capture_scope.py",
            );
        }
    }
}
