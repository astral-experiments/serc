async def captured_result(values, keys, tick):
    try:
        values = await tick()
    finally:
        await tick()
    mapping = {str(row['key']): row for row in values}
    return tuple(mapping.get(key) for key in keys)
