#[allow(dead_code)]
mod common;

#[test]
fn matches_cpython_marshal_for_coroutine_pre_suspension_any_owners() {
    let source = r"async def any_eq(values, expected, tick):
    assert any(value == expected for value in values)
    await tick()


async def any_ne(values, unexpected, tick):
    assert any(value != unexpected for value in values)
    await tick()


async def any_is(values, expected, tick):
    assert any(value is expected for value in values)
    await tick()


async def any_gt(values, tick):
    assert any(value > 0 for value in values)
    await tick()


async def any_call(values, tick):
    assert any(check(value) for value in values)
    await tick()


async def any_and(values, tick):
    assert any(value.ready and value.pending for value in values)
    await tick()


async def any_filtered(values, tick):
    assert any(value.ready for value in values if value)
    await tick()


async def any_no_later_await(values):
    assert any(value.ready for value in values)


async def any_prior_list(values, tick):
    copied = [value for value in values]
    assert any(value.ready for value in copied)
    await tick()
";
    common::assert_matches_cpython_marshal(source, "coroutine_pre_suspension_any_owners.py");
}
