async def update(manager, cache, enabled, limit, load):
    result = await load()
    async with manager:
        if enabled:
            cache["current"] = result
            while len(cache) > limit:
                cache.popitem(last=False)
    return result
