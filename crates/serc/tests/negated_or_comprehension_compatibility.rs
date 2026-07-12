#[allow(dead_code)]
mod common;

#[test]
fn matches_cpython_marshal_for_negated_or_comprehension_filters() {
    let source = r#"def generator_calls(items, first, second):
    return tuple(item for item in items if not (first(item) or second(item)))

def generator_attributes(items):
    return tuple(item for item in items if not (item.first or item.second))

def generator_comparisons(items):
    return tuple(item for item in items if not ("left" in item or "right" in item))

def generator_three(items, first, second, third):
    return tuple(item for item in items if not (first(item) or second(item) or third(item)))

def generator_multiple_filters(items, first, second, keep):
    return tuple(item for item in items if keep(item) if not (first(item) or second(item)))

def generator_multiline(items, first, second):
    return tuple(
        item
        for item in items
        if not (
            first(item)
            or second(item)
        )
    )

def generator_in_with(manager, items, first, second):
    with manager:
        return tuple(item for item in items if not (first(item) or second(item)))

def list_calls(items, first, second):
    return [item for item in items if not (first(item) or second(item))]

def set_calls(items, first, second):
    return {item for item in items if not (first(item) or second(item))}

def dict_calls(items, first, second):
    return {item: item.value for item in items if not (first(item) or second(item))}

def negated_and_control(items, first, second):
    return tuple(item for item in items if not (first(item) and second(item)))

def positive_or_control(items, first, second):
    return tuple(item for item in items if first(item) or second(item))

def positive_and_control(items, first, second):
    return tuple(item for item in items if first(item) and second(item))

def outside_control(item, first, second):
    return not (first(item) or second(item))

"#;
    common::assert_matches_cpython_marshal(source, "negated_or_comprehension_filters.py");
}
