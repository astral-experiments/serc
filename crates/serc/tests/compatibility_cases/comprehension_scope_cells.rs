use crate::common;
use crate::common::indent;

#[test]
#[ignore = "Serc reserves the captured generator target before its sibling locals"]
fn generator_inline_cell_slot_order_matches_cpython() {
    let source = r"g = (locals() for x, y in [(1, 2)] if [lambda: x for x in [1]])
print(list(next(g)))
";
    common::assert_executes(source, "['.0', 'y', 'x']\n");
    common::assert_matches_cpython_marshal(source, "generator_inline_cell_slot_order.py");
}

fn comprehension(kind: &str, element: &str, target: &str) -> String {
    match kind {
        "list" => format!("[{element} for {target} in values]"),
        "set" => format!("{{{element} for {target} in values}}"),
        "dict" => format!("{{{target}: {element} for {target} in values}}"),
        _ => unreachable!(),
    }
}

#[test]
fn matches_cpython_external_closures_in_comprehension_assignment_targets() {
    let source = r#"
def collections(key, values):
    holder = {}
    lists = [key for holder[(lambda: key)()] in values]
    sets = {key for holder[(lambda: key)()] in values}
    dictionaries = {key: key for holder[(lambda: key)()] in values}
    generators = tuple(key for holder[(lambda: key)()] in values)
    direct_cells = [(lambda: item)() for item in values]
    return lists, sets, dictionaries, generators, direct_cells, holder

print(collections("index", [10, 20]))
"#;
    common::assert_matches_cpython_marshal(source, "comprehension_assignment_closures.py");
    common::assert_executes(
        source,
        "(['index', 'index'], {'index'}, {'index': 'index'}, ('index', 'index'), [10, 20], {'index': 20})\n",
    );
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
fn matches_cpython_outer_comprehension_reads_shadowed_only_by_a_child() {
    for expression in [
        "[(item, [item for item in parts]) for outer in values]",
        "[[item for item in parts] for outer in values if item]",
        "[item for outer in values if [item for item in parts]]",
        "[[item for item in parts] for holder[item] in values]",
        "[item for holder[[item for item in parts][0]] in values]",
        "{item for outer in values if {item for item in parts}}",
        "{item: outer for outer in values if {item: item for item in parts}}",
        "[[item for item in parts] for outer in values]",
        "[(item, [item for item in parts]) for item in values]",
    ] {
        for body in [
            format!("result = {expression}\n"),
            format!("class Class:\n    result = {expression}\n"),
            format!("def function(values, parts, holder):\n    return {expression}\n"),
            format!(
                "def function(values, parts, holder):\n    item = 2\n    return {expression}\n"
            ),
            format!(
                "def function(values, parts, holder):\n    global item\n    return {expression}\n"
            ),
            format!("def function(values, parts, holder, item):\n    return {expression}\n"),
            format!(
                "def outer(item):\n    def function(values, parts, holder):\n        return {expression}\n    return function\n"
            ),
        ] {
            common::assert_matches_cpython_marshal(&body, "outer_comprehension_read.py");
        }
    }

    let source = r"
item = 0
def function(values, parts, holder):
    one = [(item, [item for item in parts]) for outer in values]
    two = [[item for item in parts] for holder[item] in values]
    three = [item for holder[[item for item in parts][0]] in values]
    return one, two, three, holder
print(function([1, 2], [1, 2], [0, 0, 0]))
";
    common::assert_executes(
        source,
        "([(0, [1, 2]), (0, [1, 2])], [[1, 2], [1, 2]], [0, 0], [2, 2, 0])\n",
    );
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
