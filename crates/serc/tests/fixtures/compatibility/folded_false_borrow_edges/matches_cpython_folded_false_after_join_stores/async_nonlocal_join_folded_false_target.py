def outer():
    captured = None
    async def choose(value):
        nonlocal captured
        captured = value.attribute if value else None
        if False:
            yield None
    return choose
