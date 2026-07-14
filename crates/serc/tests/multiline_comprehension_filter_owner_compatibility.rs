#[allow(dead_code)]
mod common;

#[test]
fn matches_cpython_marshal_for_multiline_comprehension_filter_owners() {
    let source = r#"def multiline_simple(values, suffix):
    return [
        item
        for item in values
        if suffix is None or item.name.endswith(suffix)
    ]


def multiline_cast(values, suffix, cast, make):
    return [
        cast("Item", make(name=item.name))
        for item in values
        if suffix is None or item.name.endswith(suffix)
    ]


def multiline_call_element(values, suffix, make):
    return [
        make(item.name, item.color)
        for item in values
        if suffix is None or item.name.endswith(suffix)
    ]


def same_line_control(values, suffix):
    return [item for item in values if suffix is None or item.name.endswith(suffix)]


def later_filter_control(values, suffix, ready):
    return [
        item
        for item in values
        if ready(item)
        if suffix is None or item.name.endswith(suffix)
    ]


async def coroutine_control(values, suffix):
    return [
        item
        for item in values
        if suffix is None or item.name.endswith(suffix)
    ]


def yielding_loop_control(values, suffix):
    for item in values:
        if suffix is None or item.name.endswith(suffix):
            yield item
"#;
    common::assert_matches_cpython_marshal(source, "multiline_comprehension_filter_owners.py");
}
