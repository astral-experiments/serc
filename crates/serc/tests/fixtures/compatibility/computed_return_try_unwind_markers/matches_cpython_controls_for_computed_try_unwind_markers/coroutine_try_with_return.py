async def f(manager, value):
    with manager:
        try:
            return value
        except Error:
            return None
