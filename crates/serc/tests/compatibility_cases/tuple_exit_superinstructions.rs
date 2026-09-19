use crate::common::assert_matches_cpython_marshal;

#[test]
fn matches_cpython_tuple_exit_local_fusion_in_suspended_scopes() {
    for (scope, prefix) in [
        ("def", ""),
        ("async def", ""),
        ("def", "    yield a\n"),
        ("async def", "    await warmup()\n"),
    ] {
        for extra_parameters in [
            "",
            "p0, p1, p2, p3, p4, p5, p6, p7, p8, p9, p10, p11, p12, p13, ",
        ] {
            for tail in ["c", "c, d", "c, d, e"] {
                for separator in [" ", "\n        "] {
                    let source = format!(
                        "{scope} collect({extra_parameters}flag, a, b, c, d, e):\n{prefix}    return (a if flag else b,{separator}{tail})\n"
                    );
                    assert_matches_cpython_marshal(&source, "tuple_exit_superinstructions.py");
                }
            }
        }
    }
}

#[test]
fn matches_cpython_tuple_exit_original_assignment_location_after_comprehension() {
    for (filename, source) in [
        (
            "nested_comprehension_tuple_return.py",
            r"def collect_values(enabled, direct, alternate, items, convert):
    first = None
    result = []
    if enabled:
        if direct:
            assert items
            result = [convert(items[0], flag=True)]
        else:
            result = [
                convert(
                    item,
                    flag=False,
                )
                for item in items
            ]
    elif alternate:
        result = [first]
    return first, result
",
        ),
        (
            "control_list_return.py",
            r"def collect_values(enabled, direct, alternate, items, convert):
    first = None
    result = []
    if enabled:
        if direct:
            assert items
            result = [convert(items[0], flag=True)]
        else:
            result = [
                convert(
                    item,
                    flag=False,
                )
                for item in items
            ]
    elif alternate:
        result = [first]
    return [first, result]
",
        ),
        (
            "control_no_nested_if.py",
            r"def collect_values(enabled, direct, alternate, items, convert):
    first = None
    result = []
    if enabled:
        result = [
            convert(
                item,
                flag=False,
            )
            for item in items
        ]
    elif alternate:
        result = [first]
    return first, result
",
        ),
        (
            "control_no_comprehension.py",
            r"def collect_values(enabled, direct, alternate, items, convert):
    first = None
    result = []
    if enabled:
        if direct:
            assert items
            result = [convert(items[0], flag=True)]
        else:
            result = [
                convert(
                    first,
                    flag=False,
                )
            ]
    elif alternate:
        result = [first]
    return first, result
",
        ),
    ] {
        assert_matches_cpython_marshal(source, filename);
    }
}
