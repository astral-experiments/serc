async def f(manager, values, stream):
    async with manager:
        try:
            async for value in values:
                pass
        finally:
            assert stream.metadata().get("key", None) == "expected"
            checked = True
    return checked
