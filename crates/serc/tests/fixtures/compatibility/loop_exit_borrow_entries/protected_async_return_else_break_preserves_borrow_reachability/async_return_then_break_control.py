async def route(self, stream, manager, choose, consume):
    try:
        async with manager:
            async for item in stream:
                if choose(item):
                    return item
                consume(item)
                break
            return self.fallback()
    except Exception:
        return self.fallback()
