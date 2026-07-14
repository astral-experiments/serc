use crate::common;

#[test]
fn matches_cpython_marshal_for_coroutine_optimized_boolean_loop_owners() {
    let source = r"async def post_await_assert_all(values, tick):
    await tick()
    assert all(value for value in values)


async def post_await_assert_any(values, tick):
    await tick()
    assert any(value for value in values)


async def post_await_assert_not_any(values, tick):
    await tick()
    assert not any(value for value in values)


async def post_await_filtered_assert(values, tick):
    await tick()
    assert all(value for value in values if value)


async def post_await_boolean_element(values, tick):
    await tick()
    assert any(value and value.ready for value in values)


async def post_await_assignment(values, tick):
    await tick()
    result = any(value for value in values)
    return result


async def post_await_boolean_value(values, flag, tick):
    await tick()
    result = flag and any(value for value in values)
    return result


async def post_await_ifexp(values, flag, tick):
    await tick()
    result = all(value for value in values) if flag else False
    return result


async def post_await_if(values, tick):
    await tick()
    if all(value for value in values):
        return True
    return False


async def post_await_prior_list(values, tick):
    await tick()
    copied = [value for value in values]
    assert any(value for value in copied)


async def post_await_prior_set(values, tick):
    await tick()
    copied = {value for value in values}
    assert all(value for value in copied)


async def post_await_prior_dict(values, tick):
    await tick()
    copied = {value: value for value in values}
    assert any(value for value in copied)


async def post_async_with_all(manager, values, tick):
    async with manager:
        await tick()
    assert all(value.ready for value in values)


async def post_async_with_any(manager, values, tick):
    async with manager:
        await tick()
    assert any(value.ready for value in values)


async def lambda_default_capture_all(values, tick):
    await tick()
    assert all((lambda value=value: value.ready)() for value in values)


async def lambda_default_capture_any(values, tick):
    await tick()
    assert any((lambda value=value: value.ready)() for value in values)


async def function_default_all(values, tick):
    await tick()

    def inner(result=all(value.ready for value in values)):
        return result

    return inner()


async def lambda_default_any(values, tick):
    await tick()
    assert (lambda result=any(value.ready for value in values): result)()


async def nested_generator_all(entries, tick):
    await tick()
    assert all(value.ready for entry in entries for value in entry.values)


async def nested_generator_filtered_any(entries, tick):
    await tick()
    assert any(value.ready for entry in entries if entry for value in entry.values if value)


async def nested_definition_post_suspend(values, tick):
    async def inner():
        await tick()
        assert any(value.ready for value in values)
        return values

    return await inner()


async def local_all_shadow(values, tick, all):
    await tick()
    assert all(value.ready for value in values)


async def local_any_shadow(values, tick, any):
    await tick()
    assert any(value.ready for value in values)


async def outer_await_iter_all(tick):
    await tick()
    assert all(value.ready for value in await tick())


async def outer_await_iter_any(tick):
    await tick()
    assert any(value.ready for value in await tick())


async def outer_async_list_iter_all(stream, tick):
    await tick()
    assert all(value.ready for value in [item async for item in stream])
";
    common::assert_matches_cpython_marshal(source, "coroutine_optimized_boolean_loop_owners.py");
}
