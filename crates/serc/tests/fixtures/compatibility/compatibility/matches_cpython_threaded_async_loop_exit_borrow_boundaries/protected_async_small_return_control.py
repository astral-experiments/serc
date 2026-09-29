async def run(manager, source, value):
    async with manager:
        try:
            async for item in source:
                break
        except Error:
            pass
        return value
