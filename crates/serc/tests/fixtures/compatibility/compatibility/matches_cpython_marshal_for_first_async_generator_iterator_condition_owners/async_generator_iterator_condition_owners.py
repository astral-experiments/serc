async def first_not_continue(stream):
    async for item in stream:
        if not item:
            continue
        yield item


async def first_attribute_multiple_statements(stream, consume):
    async for item in stream:
        if item.ready:
            consume(item)
        consume(item.value)
        yield item


async def first_call_break(stream, ready):
    async for item in stream:
        if ready(item):
            break
        yield item


async def first_yield_in_branch(stream):
    async for item in stream:
        if item.ready:
            yield item.value
        yield item


async def first_nested_statement(stream, consume):
    async for item in stream:
        if item.ready:
            if item.valid:
                consume(item)
        yield item


async def first_nested_async_for(groups):
    async for group in groups:
        async for item in group:
            if not item:
                continue
            yield item


def plain_for_control(values, consume):
    for item in values:
        if not item:
            continue
        consume(item)


async def coroutine_async_for_control(stream, consume):
    async for item in stream:
        if item.ready:
            consume(item)


def sync_generator_for_control(values):
    for item in values:
        if not item:
            continue
        yield item


async def async_generator_for_control(values):
    for item in values:
        if not item:
            continue
        yield item


async def async_generator_while_control(read):
    while True:
        item = read()
        if not item:
            break
        yield item


async def later_after_assignment_control(stream, consume):
    async for item in stream:
        value = item.value
        if value:
            consume(value)
    yield None


async def later_after_await_control(stream, tick, consume):
    async for item in stream:
        await tick(item)
        if item.ready:
            consume(item)
        yield item


async def later_after_yield_control(stream, consume):
    async for item in stream:
        yield item
        if item.ready:
            consume(item)


async def await_condition_control(stream, ready):
    async for item in stream:
        if await ready(item):
            continue
        yield item


async def generator_condition_control(stream):
    async for item in stream:
        if any(part.ready for part in item.parts):
            continue
        yield item


async def boolean_condition_control(stream, manager, enabled):
    async for item in stream:
        async with manager:
            if enabled and item.ready:
                continue
            yield item


async def conditional_expression_control(stream, manager, enabled):
    async for item in stream:
        async with manager:
            if item.ready if enabled else item.valid:
                continue
            yield item


async def async_with_control(stream, manager):
    async for item in stream:
        async with manager:
            if not item:
                continue
            yield item


def sync_generator_with_control(values, manager):
    with manager:
        for item in values:
            value = item.value
            if not value:
                continue
            yield value


def sync_generator_try_finally_control(values, consume):
    try:
        for item in values:
            value = item.value
            if not value:
                continue
            yield value
    finally:
        consume(values)


async def nested_boolean_async_with_control(stream, manager, enabled, consume):
    async for item in stream:
        async with manager:
            if enabled and item.ready:
                if item.valid or enabled:
                    consume(item)
            yield item
