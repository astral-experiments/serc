async def update(manager, cache, limit, load):
    result = await load()
    async with manager:
        cache["current"] = result
        while len(cache) > limit:
            cache.popitem(last=False)
    return result
