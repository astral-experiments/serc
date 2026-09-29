async def build(items, finish):
    result = {name: count for name, count in items if count}
    finish(result)
    return result
