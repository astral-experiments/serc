async def function(manager, value, fallback):
    with manager:
        match value:
            case Request():
                return await handle(value)
            case _:
                fallback(value)
