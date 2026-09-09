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
