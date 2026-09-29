async def build(items):
    return {name: count for name, count, enabled in items if enabled}
