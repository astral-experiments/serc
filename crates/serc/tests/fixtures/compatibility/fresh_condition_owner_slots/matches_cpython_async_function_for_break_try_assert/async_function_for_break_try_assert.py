async def f(values):
    value = None
    for item in values:
        if consume(item):
            value = item
            break
    try:
        assert value, "missing"
        return finish(value)
    except Error:
        return None
