#[allow(dead_code)]
mod common;

#[test]
fn matches_cpython_marshal_for_comprehension_tuple_guard_owners() {
    let cases = [
        (
            "list_clean.py",
            r"
def list_clean(outer, values):
    return [tuple(x for x in values) for y in outer]
",
        ),
        (
            "list_filter.py",
            r"
def list_filter(outer, values, flag):
    return [tuple(x for x in values) for y in outer if flag]
",
        ),
        (
            "try_list_clean.py",
            r"
class Error(Exception):
    pass


def try_list_clean(outer, values):
    try:
        return [tuple(x for x in values) for y in outer]
    except Error:
        return None
",
        ),
        (
            "with_list_clean.py",
            r"
def with_list_clean(manager, outer, values):
    with manager:
        return [tuple(x for x in values) for y in outer]
",
        ),
        (
            "list_pair.py",
            r"
def list_pair(outer, values, left, right):
    return [((left, right), tuple(x for x in values))[1] for y in outer]
",
        ),
        (
            "list_call.py",
            r"
def list_call(outer, values):
    return [(seed(), tuple(x for x in values))[1] for y in outer]
",
        ),
        (
            "dict_control.py",
            r"
def dict_control(outer, values):
    return {y: tuple(x for x in values) for y in outer}
",
        ),
        (
            "direct_generator_control.py",
            r"
def direct_generator_control(values):
    yield tuple(x for x in values)
",
        ),
    ];

    for (filename, source) in cases {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
