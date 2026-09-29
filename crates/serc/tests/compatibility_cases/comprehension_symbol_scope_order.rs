use crate::common;

#[test]
fn preserves_comprehension_body_references_before_later_iteration_targets() {
    for expression in [
        "[lookup(value) for value in values]",
        "{lookup(value): value for value in values}",
        "[value for value in values if lookup(value)]",
        "[other for value in values for other in lookup(value)]",
        "[[lookup(other) for other in value] for value in values]",
        "[(lambda result=lookup(value): result) for value in values]",
    ] {
        for (parameters, declaration, captures_outer) in [
            ("values", "", false),
            ("values, lookup", "", false),
            ("values", "    lookup = make_lookup()\n", false),
            ("values", "", true),
        ] {
            let function = format!(
                "def function({parameters}):\n{declaration}    result = {expression}\n    shadowed = [lookup for lookup in values]\n    return result, shadowed\n"
            );
            let source = if captures_outer {
                let indented = function.trim_end().replace('\n', "\n    ");
                format!("def enclosing(lookup):\n    {indented}\n    return function\n")
            } else {
                function
            };
            common::assert_matches_cpython_marshal(&source, "comprehension_symbol_scope_order.py");
        }
    }
}

#[test]
fn preserves_earlier_comprehension_bindings_and_outer_references() {
    for body in [
        "shadowed = [lookup for lookup in values]\nresult = [lookup(value) for value in values]",
        "shadowed = [lookup for lookup in values]\nresult = [value for value in values if lookup(value)]",
        "result = [lookup(value) for lookup in callbacks for value in values]\nshadowed = [lookup for lookup in values]",
        "result = [[lookup(value) for value in values] for lookup in callbacks]\nshadowed = [lookup for lookup in values]",
        "original = lookup\nshadowed = [lookup for lookup in values]\nresult = [lookup(value) for value in values]",
        "result = [lookup for lookup in lookup(values)]\nshadowed = [lookup for lookup in values]",
    ] {
        let indented = body.replace('\n', "\n        ");
        let source = format!(
            "def enclosing(lookup):\n    def function(values, callbacks):\n        {indented}\n        return result, shadowed\n    return function\n"
        );
        common::assert_matches_cpython_marshal(&source, "comprehension_symbol_scope_order.py");
    }
}

#[test]
fn preserves_captured_comprehension_targets_in_free_variable_slots() {
    for target in [
        "[(lambda: lookup) for lookup in values]",
        "{lookup: (lambda: lookup) for lookup in values}",
        "[[(lambda: lookup) for lookup in values] for item in values]",
        "[[(lambda: lookup) for lookup in lookup(values)] for item in values]",
    ] {
        for declaration in ["", "        nonlocal lookup\n"] {
            let source = format!(
                "def enclosing(lookup):\n    def function(values):\n{declaration}        result = [lookup(value) for value in values]\n        target = {target}\n        return result, target\n    return function\n"
            );
            common::assert_matches_cpython_marshal(&source, "comprehension_free_cell.py");
        }
    }
}
