use crate::common;

#[test]
fn class_annotations_pass_enclosing_cells_to_nested_expressions() {
    for future in ["", "from __future__ import annotations\n"] {
        for annotation in [
            "lambda: value",
            "Factory(callback=lambda arg: value)",
            "lambda value: value",
            "lambda arg=value: arg",
            "lambda: (lambda: value)",
            "(value for item in items)",
            "(item for item in value)",
        ] {
            for shadow in ["", "        value = 2\n"] {
                let source = format!(
                    "{future}def outer(value, items):\n    class C:\n{shadow}        field: {annotation}\n    return C\n"
                );
                common::assert_matches_cpython_marshal(&source, "class_annotation_captures.py");
            }
        }
    }
}

#[test]
fn class_generators_capture_enclosing_names_across_class_bindings() {
    for expression in [
        "(value for item in items)",
        "(item for item in value)",
        "((lambda: value) for item in items)",
        "[(value for item in items) for value in ()]",
    ] {
        for shadow in ["", "        value = 2\n"] {
            let source = format!(
                "def outer(value, items):\n    class C:\n{shadow}        field = {expression}\n    return C\n"
            );
            common::assert_matches_cpython_marshal(&source, "class_generator_captures.py");
        }
    }
}
