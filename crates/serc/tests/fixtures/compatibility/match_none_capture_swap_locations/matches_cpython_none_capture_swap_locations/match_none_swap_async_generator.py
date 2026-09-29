async def f(value, values):
    yield value
    with manager():
        match value:
            case (t, None, u):
                observe(t, u)
            case _:
                observe(value)
