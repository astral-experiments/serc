async def value(manager):
    async with manager as result:
        return result.value

async def constant(manager):
    async with manager:
        return 1

async def nested(outer, inner):
    async with outer:
        with inner:
            return value
