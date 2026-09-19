use crate::common;

#[test]
fn matches_cpython_tuple_membership_at_the_stack_use_limit() {
    for length in [29, 30, 31, 64] {
        let elements = (0..length)
            .map(|index| format!("'item_{index}'"))
            .collect::<Vec<_>>()
            .join(", ");
        let source = format!(
            "def constant_membership(item, value, values):\n    return item in ({elements},)\n\ndef dynamic_membership(item, value, values):\n    return item not in ({elements}, value)\n\ndef unpacked_membership(item, value, values):\n    return item in ({elements}, *values)\n\ndef asserted_membership(item, value, values):\n    assert item in ({elements},)\n\ndef iteration_control(item, value, values):\n    for element in ({elements},):\n        consume(element)\n"
        );
        common::assert_matches_cpython_marshal(&source, "tuple_membership_stack_limit.py");
    }
}

#[test]
fn matches_cpython_literal_membership_at_the_end_of_comparison_chains() {
    for length in [2, 30, 31] {
        let elements = (0..length)
            .map(|index| index.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        for (opening, closing) in [("(", ",)"), ("[", "]"), ("{", "}")] {
            let literal = format!("{opening}{elements}{closing}");
            let source = format!(
                "def final_membership(item, lower, other):\n    return lower < item in {literal}\n\ndef middle_membership(item, lower, other):\n    return item in {literal} == other\n\ndef conditional_membership(item, lower, other):\n    if lower < item in {literal}:\n        consume(item)\n\ndef filtered_membership(values):\n    return [item for item in values if 0 < item in {literal}]\n\ndef negated_filtered_membership(values):\n    return (item for item in values if not (0 < item in {literal}))\n"
            );
            common::assert_matches_cpython_marshal(&source, "chained_literal_membership.py");
        }
    }
}

#[test]
fn matches_cpython_tuple_membership_in_protected_suspended_functions() {
    let elements = (0..31)
        .map(|index| index.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let source = format!(
        "async def coroutine(item, value, values):\n    await ready()\n    try:\n        answer = item in ({elements},)\n        consume(answer)\n    except Error:\n        recover()\n\ndef generator(item, value, values):\n    yield value\n    with manager():\n        assert item not in ({elements}, *values)\n\nasync def async_generator(item, value, values):\n    await ready()\n    yield value\n    with manager():\n        if item in ({elements}, value):\n            consume(item)\n        consume(value)\n"
    );
    common::assert_matches_cpython_marshal(&source, "protected_tuple_membership.py");
}
