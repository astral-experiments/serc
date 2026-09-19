use crate::common;

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
fn matches_cpython_class_comprehension_cells_and_nested_flags() {
    for name in ["x", "__key"] {
        for value in [
            format!("lambda: {name}"),
            "lambda: 1".to_string(),
            format!("({name} for unused in ())"),
        ] {
            for expression in [
                format!("[{value} for {name} in ()]"),
                format!("{{{value} for {name} in ()}}"),
                format!("{{{name}: {value} for {name} in ()}}"),
            ] {
                let class = format!("class C:\n    items = {expression}\n");
                for source in [
                    class.clone(),
                    format!(
                        "def outer():\n    {name} = 'outside'\n{}    return C\n",
                        indent(&class),
                    ),
                    format!("class Outer:\n{}", indent(&class)),
                    class.replace("class C:", "class C[T]:"),
                ] {
                    common::assert_matches_cpython_marshal(&source, "class_comprehension_cells.py");
                }
            }
        }
    }
}

#[test]
fn matches_cpython_discarded_class_comprehension_interning() {
    for expression in [
        "[lambda: (__key, '_C__key') for __key in ()]",
        "{lambda: (__key, '_C__key') for __key in ()}",
        "{__key: lambda: (__key, '_C__key') for __key in ()}",
    ] {
        let class = format!("class C:\n    items = {expression}\n");
        for mut source in [
            class.clone(),
            format!("class C:\n    if False:\n        items = {expression}\n"),
            format!("if False:\n{}", indent(&class)),
        ] {
            source.push_str(
                "def first(value):\n    return value in {'_C__key', 'novel_marker'}\n\
                 def second(value):\n    'Distinct constants tuple.'\n    return value in {'_C__key', 'novel_marker'}\n",
            );
            common::assert_matches_cpython_marshal(&source, "discarded_class_comprehension.py");
        }
    }
}

#[test]
fn matches_cpython_class_comprehension_cell_boundaries() {
    for name in ["x", "__key"] {
        for outside in [
            format!("lambda: {name}"),
            format!("({name} for unused in ())"),
        ] {
            for before in [false, true] {
                let comprehension = format!("items = [lambda: {name} for {name} in ()]\n");
                let other = format!("other = {outside}\n");
                let body = if before {
                    other + &comprehension
                } else {
                    comprehension + &other
                };
                let source = format!("class C:\n{}", indent(&body));
                common::assert_matches_cpython_marshal(&source, "class_comprehension_boundary.py");
            }
        }
    }

    for before in [false, true] {
        let comprehension = "items = [lambda: x for x in ()]\n";
        let other = "other = lambda: x\n";
        let body = if before {
            other.to_string() + comprehension
        } else {
            comprehension.to_string() + other
        };
        let class = format!("class C:\n{}", indent(&body));
        let source = format!(
            "def outer():\n    x = 'outside'\n{}    return C\n",
            indent(&class),
        );
        common::assert_matches_cpython_marshal(&source, "class_comprehension_free_and_cell.py");
    }
}
