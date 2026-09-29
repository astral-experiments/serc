async def simple(manager):
    async with manager() as value:
        await value()

async def returned(manager):
    async with manager() as value:
        result = await value()
    return result

async def preceded(manager, item):
    async with manager() as value:
        value.prepare(item)
        result = await value()
    return result

async def two_calls(manager, item):
    initial = item.prepare()
    async with manager() as value:
        value.first(initial)
        value.second(initial, flag=True)
        left, right = await value()
    return left

async def tuple_target(manager):
    async with manager() as (first, second):
        result = await second(first)
    return result
