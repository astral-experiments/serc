use crate::common;

#[test]
fn matches_cpython_marshal_for_try_loop_attribute_ifexp_owners() {
    let cases = [
        (
            "repeated_attribute.py",
            r#"
class Error(Exception):
    pass


def repeated_attribute(items):
    results = []
    for index, item in enumerate(items, start=1):
        if not item:
            return None
        try:
            results.append(
                {
                    "name": item.name if item.name else f"item{index}",
                    "payload": item.payload(),
                }
            )
        except Error:
            return None
    return results
"#,
        ),
        (
            "ordinary_try_control.py",
            r#"
class Error(Exception):
    pass


def ordinary_try_control(item):
    try:
        return item.name if item.name else "fallback"
    except Error:
        return None
"#,
        ),
        (
            "different_attribute_control.py",
            r#"
class Error(Exception):
    pass


def different_attribute_control(items):
    for item in items:
        try:
            use(item.name if item.title else "fallback")
        except Error:
            continue
"#,
        ),
        (
            "different_object_control.py",
            r#"
class Error(Exception):
    pass


def different_object_control(items, other):
    for item in items:
        try:
            use(item.name if other.name else "fallback")
        except Error:
            continue
"#,
        ),
        (
            "nested_attribute_control.py",
            r#"
class Error(Exception):
    pass


def nested_attribute_control(items):
    for item in items:
        try:
            use(item.meta.name if item.meta.name else "fallback")
        except Error:
            continue
"#,
        ),
        (
            "concrete_pair_control.py",
            r#"
class Error(Exception):
    pass


def concrete_pair_control(items, left, right):
    for item in items:
        try:
            pair = (left, right)
            use(item.name if item.name else "fallback")
        except Error:
            return pair
"#,
        ),
    ];

    for (filename, source) in cases {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
