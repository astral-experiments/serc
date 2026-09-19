use crate::common;

#[test]
fn matches_cpython_lambda_comprehension_outer_references() {
    for expression in [
        "[key for key in select(key)]",
        "{key for key in select(key)}",
        "{key: value for key, value in select(key)}",
        "[key for key in key(values)]",
        "(key, [key for key in values])",
        "([key for key in values], key)",
        "[key for key in [key for key in select(key)]]",
        "[key for key in (lambda data=select(key): data)()]",
        "[key for key in select(key or values)]",
        "[key for key in select(key) if predicate(key)]",
    ] {
        for nested in [false, true] {
            let source = format!(
                "def outer(key):\n    return lambda values: {}{expression}\n",
                if nested { "(lambda: " } else { "" },
            );
            let source = if nested {
                format!("{})\n", source.trim_end())
            } else {
                source
            };
            common::assert_matches_cpython_marshal(
                &source,
                "lambda_comprehension_outer_references.py",
            );
        }
    }
}

#[test]
fn preserves_lambda_comprehension_binding_scopes() {
    for expression in [
        "[key for key in select(key)]",
        "[key for key in (lambda: select(key))()]",
        "[key for key in select(key) for other in values]",
    ] {
        for lambda in [
            format!("lambda values, key: {expression}"),
            format!("lambda values, key=key: {expression}"),
            format!("lambda values: ((key := values), {expression})"),
            format!("lambda values: {expression}"),
        ] {
            let source = format!("def outer(key):\n    return {lambda}\n");
            common::assert_matches_cpython_marshal(&source, "lambda_comprehension_bindings.py");
        }
    }
}
