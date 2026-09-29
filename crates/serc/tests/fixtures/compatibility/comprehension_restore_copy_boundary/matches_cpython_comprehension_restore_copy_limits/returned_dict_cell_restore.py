async def build(items):
    return {name: (lambda: count) for name, count in items}
