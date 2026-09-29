async def plain(stream):
    values = [value async for value in stream]
    return values


async def attribute_filter(stream):
    values = [value async for value in stream if value.ready]
    return values


async def method_filter(stream):
    values = [value async for value in stream if value.ready()]
    return values


async def comparison_filter(stream, expected):
    values = [value async for value in stream if value.kind == expected]
    return values


async def conjunction_filter(stream, expected):
    values = [value async for value in stream if value.kind == expected and value.ready]
    return values


async def disjunction_filter(stream, expected):
    values = [value async for value in stream if value.kind == expected or value.ready]
    return values


async def returned(stream):
    return [value async for value in stream if value.ready]


async def set_result(stream):
    values = {value.name async for value in stream if value.ready}
    return values


async def dict_result(stream):
    values = {value.name: value async for value in stream if value.ready}
    return values


async def initialized_target(stream):
    value = None
    values = [value async for value in stream if value.ready]
    return values, value


async def prior_assignment(factory, expected):
    stream = factory()
    values = [value async for value in stream if value.kind == expected]
    return values


async def prior_call(stream, expected, touch):
    touch(stream)
    values = [value async for value in stream if value.kind == expected]
    return values


async def prior_if(stream, expected, gate):
    if gate:
        return []
    values = [value async for value in stream if value.kind == expected]
    return values


async def async_generator(stream, expected):
    values = [value async for value in stream if value.kind == expected]
    for value in values:
        yield value


async def sync_with(stream, expected, manager):
    with manager:
        values = [value async for value in stream if value.kind == expected]
    return values


async def await_iter_control(factory, expected):
    values = [value async for value in await factory() if value.kind == expected]
    return values


async def async_with_control(stream, expected, manager):
    async with manager:
        values = [value async for value in stream if value.kind == expected]
    return values


async def sync_filter_control(values, expected):
    return [value for value in values if value.kind == expected]


async def async_generator_expression_control(stream, expected):
    return (value async for value in stream if value.kind == expected)
