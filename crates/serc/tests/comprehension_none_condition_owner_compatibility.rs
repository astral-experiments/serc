#[allow(dead_code)]
mod common;

#[test]
fn matches_cpython_marshal_for_unfiltered_comprehension_none_condition_owners() {
    let cases = [
        (
            "unfiltered_comprehension_none_owners.py",
            r"
def name_is_none(a0, a1, a2, a3, a4, a5, a6, a7, a8, a9, a10, a11, a12, a13, a14, a15, values, replacement):
    return [replacement if value is None else value for value in values]


def name_is_not_none(a0, a1, a2, a3, a4, a5, a6, a7, a8, a9, a10, a11, a12, a13, a14, a15, values, replacement):
    return [value if value is not None else replacement for value in values]


def call_and_subscript(a0, a1, a2, a3, a4, a5, a6, a7, a8, a9, a10, a11, a12, a13, a14, a15, values, mapping, iterator):
    return [next(iterator) if value is None else mapping[value] for value in values]


def nested_truthiness(a0, a1, a2, a3, a4, a5, a6, a7, a8, a9, a10, a11, a12, a13, a14, a15, values, mapping, ready):
    return [(mapping[value] if value is not None else None) if ready else value for value in values]
",
        ),
        (
            "preceding_comprehension_filter_control.py",
            r"
def preceding_filter(values, replacement):
    return [replacement if value is None else value for value in values if value]
",
        ),
        (
            "folded_comprehension_filter_controls.py",
            r"
def folded_filter_in_with(fields, fallback, manager):
    with manager:
        return [(details.value if details.ready else (name if name is None else fallback)) for name, details in fields for extra in details.values if True]


async def folded_tuple_filter_in_coroutine(fields, fallback):
    return {name: (details.value if details.ready else (name if name is None else fallback)) for name, details in fields if (1,)}
",
        ),
    ];

    for (filename, source) in cases {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
