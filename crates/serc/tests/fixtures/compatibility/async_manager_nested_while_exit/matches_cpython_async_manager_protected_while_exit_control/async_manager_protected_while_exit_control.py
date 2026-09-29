async def update(manager, cache, enabled, limit, recover):
    async with manager:
        if enabled:
            try:
                while len(cache) > limit:
                    cache.popitem(last=False)
            except LookupError:
                recover()
    return cache
