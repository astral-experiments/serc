use crate::common;

#[test]
fn matches_cpython_marshal_for_synchronous_with_none_ifexp_owners() {
    let cases = [
        (
            "sync_with_name_is_not_none.py",
            r"
def choose(manager, value, present, absent):
    with manager:
        result = present if value is not None else absent
    return result
",
        ),
        (
            "sync_with_name_is_none.py",
            r"
def choose(manager, value, present, absent):
    with manager:
        result = absent if value is None else present
    return result
",
        ),
        (
            "sync_with_attribute_is_not_none.py",
            r"
def choose(manager, record, present, absent):
    with manager:
        result = present if record.value is not None else absent
    return result
",
        ),
        (
            "sync_with_subscript_is_not_none.py",
            r"
def choose(manager, record, present, absent):
    with manager:
        result = present if record[2] is not None else absent
    return result
",
        ),
        (
            "sync_with_multiple_none_ifexp.py",
            r"
def choose(manager, first, second, present, absent):
    with manager:
        left = present if first is not None else absent
        right = absent if second is None else present
    return left, right
",
        ),
        (
            "sync_with_nested_none_ifexp.py",
            r"
def choose(outer, inner, value, present, absent):
    with outer:
        with inner:
            result = present if value is not None else absent
    return result
",
        ),
        (
            "generator_sync_with_none_ifexp_control.py",
            r"
def choose(manager, value, present, absent):
    with manager:
        yield present if value is not None else absent
",
        ),
        (
            "no_with_none_ifexp_control.py",
            r"
def choose(value, present, absent):
    result = present if value is not None else absent
    return result
",
        ),
        (
            "sync_with_equality_control.py",
            r"
def choose(manager, value, present, absent):
    with manager:
        result = present if value == None else absent
    return result
",
        ),
        (
            "sync_with_reversed_none_control.py",
            r"
def choose(manager, value, present, absent):
    with manager:
        result = present if None is not value else absent
    return result
",
        ),
        (
            "sync_with_other_compare_control.py",
            r"
def choose(manager, value, present, absent):
    with manager:
        result = present if value is not absent else absent
    return result
",
        ),
        (
            "sync_with_chained_compare_control.py",
            r"
def choose(manager, value, other, present, absent):
    with manager:
        result = present if value is not None is not other else absent
    return result
",
        ),
        (
            "sync_with_bool_control.py",
            r"
def choose(manager, value, other, present, absent):
    with manager:
        result = present if value is not None and other else absent
    return result
",
        ),
    ];

    for (filename, source) in cases {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
