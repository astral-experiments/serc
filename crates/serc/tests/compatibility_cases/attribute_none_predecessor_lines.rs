use crate::common;

#[test]
fn matches_cpython_marshal_for_none_operand_predecessor_lines() {
    for (filename, source) in [
        (
            "attribute_if_none.py",
            r#"def f(fetch, use):
    if fetch(
        "key"
    ).value is None:
        use()
"#,
        ),
        (
            "await_attribute_assert_none.py",
            r"async def f(fetch):
    assert (
        await fetch()
    ).value is None
",
        ),
        (
            "await_attribute_try_finally_control.py",
            r"async def f(fetch, cleanup):
    try:
        assert (
            await fetch()
        ).value is not None
    finally:
        cleanup()
",
        ),
        (
            "await_attribute_while_or_none.py",
            r"async def f(fetch, other, pause, manager):
    async with manager:
        while (await fetch())[
            0
        ].value is None or other()[
            0
        ].value is None:
            await pause()
",
        ),
        (
            "await_subscript_attribute_assert_not_none.py",
            r#"async def f(fetch):
    assert (await fetch())[
        "key"
    ].value is not None
"#,
        ),
        (
            "empty_multiline_free_call_control.py",
            r"def f(fetch):
    if fetch(
    ) is None:
        return 1
    return 2
",
        ),
        (
            "attribute_none_distinct_line_control.py",
            r"async def f(fetch):
    assert (
        (await fetch()).value
        is None
    )
",
        ),
        (
            "conditional_operand_none_control.py",
            r"def f(flag, left, right):
    if (
        left()
        if flag
        else right()
    ) is None:
        return 1
    return 2
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
