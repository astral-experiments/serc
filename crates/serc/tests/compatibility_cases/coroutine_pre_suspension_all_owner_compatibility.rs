use crate::common;

#[test]
fn matches_cpython_marshal_for_coroutine_pre_suspension_all_owners() {
    let source = r"async def all_eq(values, expected, tick):
    assert all(value == expected for value in values)
    await tick()


async def all_ne(values, unexpected, tick):
    assert all(value != unexpected for value in values)
    await tick()


async def all_is(values, expected, tick):
    assert all(value is expected for value in values)
    await tick()


async def all_gt(values, tick):
    assert all(value > 0 for value in values)
    await tick()


async def all_call(values, tick):
    assert all(check(value) for value in values)
    await tick()


async def all_or(values, tick):
    assert all(value.ready or value.pending for value in values)
    await tick()


async def all_filtered(values, tick):
    assert all(value.ready for value in values if value)
    await tick()


async def all_no_later_await(values):
    assert all(value.ready for value in values)


async def all_prior_list(values, tick):
    copied = [value for value in values]
    assert all(value.ready for value in copied)
    await tick()
";
    common::assert_matches_cpython_marshal(source, "coroutine_pre_suspension_all_owners.py");
}
