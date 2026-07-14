#[allow(dead_code)]
mod common;

#[test]
fn matches_cpython_marshal_for_coroutine_pre_suspension_not_any_owners() {
    let source = r#"async def not_any_eq(values, expected, tick):
    assert not any(value == expected for value in values)
    await tick()


async def not_any_in(values, expected, tick):
    assert not any(value in expected for value in values)
    await tick()


async def not_any_and(values, tick):
    assert not any(value.ready and value.pending for value in values)
    await tick()


async def not_any_filtered(values, tick):
    assert not any(value.ready for value in values if value)
    await tick()


async def not_any_name(values, tick):
    assert not any(value for value in values)
    await tick()


async def not_any_string_message(values, expected, tick):
    assert not any(value == expected for value in values), "unexpected value"
    await tick()


async def not_any_fstring_message(values, expected, tick):
    assert not any(value == expected for value in values), f"unexpected {expected!r}"
    await tick()


async def not_any_tuple_message(values, expected, tick):
    assert not any(value == expected for value in values), (
        "unexpected value " f"{expected!r}"
    )
    await tick()


async def not_any_call_message(values, expected, message, tick):
    assert not any(value == expected for value in values), message(expected)
    await tick()


async def not_any_no_later_await(values, expected):
    assert not any(value == expected for value in values)
"#;
    common::assert_matches_cpython_marshal(source, "coroutine_pre_suspension_not_any_owners.py");
}
