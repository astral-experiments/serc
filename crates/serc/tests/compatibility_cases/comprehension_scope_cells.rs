use crate::common;

fn comprehension(kind: &str, element: &str, target: &str) -> String {
    match kind {
        "list" => format!("[{element} for {target} in values]"),
        "set" => format!("{{{element} for {target} in values}}"),
        "dict" => format!("{{{target}: {element} for {target} in values}}"),
        _ => unreachable!(),
    }
}

fn indent(source: &str) -> String {
    let mut result = String::new();
    for line in source.lines() {
        result.push_str("    ");
        result.push_str(line);
        result.push('\n');
    }
    result
}

#[test]
fn matches_cpython_child_globals_outside_comprehension_targets() {
    for kind in ["list", "set", "dict"] {
        for element in ["item", "(lambda: item)"] {
            for child in [
                "child = lambda: item\n",
                "def child():\n    return item\n",
                "class child:\n    @item\n    def method(self):\n        pass\n",
            ] {
                for prefix in ["", "global item\n"] {
                    for before in [false, true] {
                        let expression =
                            format!("result = {}\n", comprehension(kind, element, "item"));
                        let body = if before {
                            format!("{prefix}{expression}{child}return child\n")
                        } else {
                            format!("{prefix}{child}{expression}return child\n")
                        };
                        let source = format!("def outer(values, parts):\n{}", indent(&body));
                        common::assert_matches_cpython_marshal(&source, "comprehension_globals.py");
                    }
                }
            }
        }
    }
}

#[test]
fn matches_cpython_captured_comprehension_cells_in_module_headers() {
    for kind in ["list", "set", "dict"] {
        for element in ["(lambda: item)", "tuple(item for part in parts)"] {
            let expression = comprehension(kind, element, "item");
            for header in [
                "@decorate(EXPR)\ndef f():\n    pass\n",
                "def f(value=EXPR):\n    pass\n",
                "def f(*, value=EXPR):\n    pass\n",
                "@decorate(EXPR)\nclass C:\n    pass\n",
                "class C(*EXPR):\n    pass\n",
                "class C(metaclass=make(EXPR)):\n    pass\n",
            ] {
                let source = format!(
                    "{}external = lambda: item\n",
                    header.replace("EXPR", &expression)
                );
                common::assert_matches_cpython_marshal(&source, "module_header_cells.py");
            }
        }
    }
}

#[test]
fn matches_cpython_generator_comprehension_fast_cell_order() {
    for kind in ["list", "set", "dict"] {
        for (outer, target) in [("aa", "zz"), ("zz", "aa")] {
            for capture_outer in [false, true] {
                for lambda in [false, true] {
                    for position in ["element", "filter", "later_iter", "first_iter"] {
                        let value = if capture_outer && position != "first_iter" {
                            format!("({outer}, {target})")
                        } else {
                            target.to_string()
                        };
                        let element = if lambda {
                            format!("(lambda: {value})")
                        } else {
                            format!("tuple({value} for part in parts)")
                        };
                        let expression = comprehension(kind, &element, target);
                        let generator = match position {
                            "element" => format!("({expression} for {outer} in values)"),
                            "filter" => format!("({outer} for {outer} in values if {expression})"),
                            "later_iter" => {
                                format!("({outer} for {outer} in values for tail in {expression})")
                            }
                            "first_iter" => format!("({outer} for {outer} in {expression})"),
                            _ => unreachable!(),
                        };
                        let source = format!("def f(values, parts):\n    return {generator}\n");
                        common::assert_matches_cpython_marshal(&source, "generator_fast_cells.py");
                    }
                }
            }
        }
    }
}

#[test]
fn matches_cpython_nested_comprehension_target_restoration() {
    for kind in ["list", "set", "dict"] {
        for inner in [
            "[item for item in parts]",
            "[lambda: item for item in parts]",
        ] {
            let expression = comprehension(kind, &format!("({inner}, lambda: item)"), "item");
            for template in [
                "result = EXPR\n",
                "def function(result=EXPR):\n    return result\n",
                "def function(values, parts):\n    return EXPR\n",
                "function = lambda: EXPR\n",
            ] {
                common::assert_matches_cpython_marshal(
                    &template.replace("EXPR", &expression),
                    "nested_target_restoration.py",
                );
            }
        }
    }
}

#[test]
fn matches_cpython_comprehension_temporaries_in_type_parameter_thunks() {
    for kind in ["list", "set", "dict"] {
        for element in ["item", "(lambda: item)"] {
            let expression = comprehension(kind, element, "item");
            let inner = comprehension(kind, "inner", "inner");
            for expression in [
                expression.replace("in values", &format!("in {inner}")),
                comprehension(kind, &format!("({expression}, lambda: outer)"), "outer"),
                comprehension(kind, &format!("({expression}, lambda: item)"), "item"),
                format!("(item, {expression})"),
                format!("({expression}, item)"),
                expression.replace("in values", "in item"),
            ] {
                for template in [
                    "def function[T: EXPR]():\n    pass\n",
                    "def function[T = EXPR]():\n    pass\n",
                    "class Class[T: EXPR]:\n    pass\n",
                    "class Class[T = EXPR]:\n    pass\n",
                ] {
                    common::assert_matches_cpython_marshal(
                        &template.replace("EXPR", &expression),
                        "type_parameter_comprehension.py",
                    );
                }
            }
        }
    }
}

#[test]
fn matches_cpython_nested_global_declarations_before_module_comprehensions() {
    for (filename, source) in [
        (
            "module_comprehension_nested_global_read_target.py",
            r#"def configure():
    global needle
    needle = "p"
before = needle
result = [value for value in values_for(needle) if needle in value and value.startswith("p_")]
after = needle
"#,
        ),
        (
            "module_comprehension_nested_global_callable_target.py",
            r"def configure():
    global check
    check = predicate
result = [check(value) for value in values]
after = check()
",
        ),
        (
            "module_comprehension_nested_walrus_global_control.py",
            r"def configure():
    global captured
    captured = None
result = [(captured, [(captured := value) for value in values], captured) for outer in groups]
after = captured
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
