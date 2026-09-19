use crate::common;

#[test]
fn matches_cpython_marshal_for_tuple_unpack_owner_slots() {
    for prefix in [
        "    if flag:\n        return None\n",
        "    for item in items:\n        pass\n",
    ] {
        for assignment in [
            "holder.first, = first,",
            "holder.first, holder.second = first, second",
            "holder.first, holder.second, holder.third = first, second, third",
            "a, = first,",
            "a, b = first, second",
            "a, b, c = first, second, third",
            "holder.first, = (first if flag else second),",
            "a, = (first if flag else second),",
            "holder.first, holder.second = 1, 2",
            "a, b, c = 1, 2, 3",
            "holder.first = first",
        ] {
            let source = format!(
                "async def unpack(holder, first, second, third, value, fallback, flag, items):
{prefix}    {assignment}
    return holder.select(first, third, value or fallback, second)
"
            );
            common::assert_matches_cpython_marshal(&source, "tuple_unpack_owner_slots.py");
        }
    }
}

#[test]
fn matches_cpython_marshal_for_multiline_tuple_unpack_locations() {
    for (filename, source) in [
        (
            "multiline_unpack_two_values.py",
            r"def target(value):
    (
        left,
        right,
    ) = (
        value,
        [],
    )
    return left, right
",
        ),
        (
            "multiline_unpack_three_constants.py",
            r"def target():
    (
        first,
        second,
        third,
    ) = (
        None,
        None,
        None,
    )
    return first, second, third
",
        ),
        (
            "multiline_unpack_module_calls.py",
            r"(
    first,
    second,
) = (
    make(value=1),
    make(value=2),
)
",
        ),
        (
            "multiline_unpack_rhs_only_control.py",
            r"def target(value):
    left, right = (
        value,
        [],
    )
    return left, right
",
        ),
        (
            "multiline_unpack_single_line_control.py",
            r"def target(value):
    left, right = value, []
    return left, right
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
