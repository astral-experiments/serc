async def discarded():
    [item async for item in source]

async def returned():
    return [item async for item in source]
