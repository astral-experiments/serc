async def f(values):
    value = None
    for item in values:
        if consume(item):
            value = item
            break
    try:
        result = finish(value) if value else None
    except Error:
        return None
    return result
