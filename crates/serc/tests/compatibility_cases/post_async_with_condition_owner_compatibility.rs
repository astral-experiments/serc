use crate::common;

#[test]
fn matches_cpython_marshal_for_post_async_with_condition_owners() {
    let source = r"async def direct_assert(manager, holder, use):
    async with manager:
        use(holder)
    assert holder.ready()
    use(holder)


async def negated_assert(manager, holder, use):
    async with manager:
        pass
    assert not holder.closed
    use(holder)


async def short_circuit_if(manager, holder, flag, use):
    async with manager:
        use(holder)
    if not holder.closed and flag:
        use(holder)
    use(flag)


async def nested_managers(manager, other, holder, pause, use):
    async with manager:
        async with other:
            await pause()
    assert holder.ready()
    use(holder)


async def outer_sync_manager(manager, sync_manager, holder, use):
    with sync_manager:
        async with manager:
            use(holder)
        if holder.ready():
            use(holder)
        use(holder)


async def async_generator(manager, holder, use):
    async with manager:
        use(holder)
    assert not holder.closed
    yield holder


async def sequential_managers(manager, other, holder, pause, use):
    async with manager:
        use(holder)
    async with other:
        await pause()
    assert holder.ready()
    use(holder)


async def global_call_control(manager, value, use):
    async with manager:
        use(value)
    assert isinstance(value, object)
    use(value)


async def fused_load_control(manager, left, right, use):
    async with manager:
        use(left)
    assert (left / right).ready()
    use(left)


async def none_control(manager, value, use):
    async with manager:
        use(value)
    assert value is not None
    use(value)


async def comparison_control(manager, value, flag, use):
    async with manager:
        use(value)
    if value == flag:
        use(value)
    use(flag)


async def chained_control(manager, value, flag, limit, use):
    async with manager:
        use(value)
    assert flag < value < limit
    use(value)


async def await_control(manager, value, pause, use):
    async with manager:
        use(value)
    await pause()
    assert value
    use(value)


async def sync_bridge_control(manager, sync_manager, value, use):
    async with manager:
        use(value)
    with sync_manager:
        use(value)
    assert value
    use(value)


async def nested_suite_reset_control(manager, value, flag, use):
    async with manager:
        use(value)
    if value is None:
        if flag:
            use(flag)
    use(value)


async def nested_definition_reset_control(manager, value, use):
    async with manager:
        use(value)

    def inner(flag):
        if flag:
            use(flag)
        return flag

    return inner(value)


async def loop_control(manager, values, use):
    for value in values:
        async with manager:
            use(value)
        assert value
    use(values)


async def post_async_while_none_interaction(manager, sync_manager, value, use, advance):
    async with manager:
        with sync_manager:
            while value is not None:
                use(value)
                value = advance(value)
    assert value
    use(value)


async def post_async_awaited_while_none_interaction(manager, cursor, value, use):
    async with manager:
        while (await cursor.fetchone()) is not None:
            use(value)
    if value:
        use(value)
    use(cursor)
";
    common::assert_matches_cpython_marshal(source, "post_async_with_condition_owners.py");
}
