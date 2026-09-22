async def coroutine(awaitable):
    return await awaitable


async def async_tail(items, result):
    async for value in items:
        if value:
            result.append(value)


async def protected_async_for(manager, items):
    with manager:
        async for item in items:
            ...


async def protected_async_comprehension(manager, items):
    async with manager:
        consume({item async for item in items})
