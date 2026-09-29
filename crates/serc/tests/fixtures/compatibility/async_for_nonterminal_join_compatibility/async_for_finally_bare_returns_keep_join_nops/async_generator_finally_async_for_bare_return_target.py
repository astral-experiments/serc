async def target(enabled, items):
    if enabled:
        try:
            try:
                async for item in items:
                    pass
            except Exception:
                pass
            yield make_value()
        finally:
            cleanup = items
            async for item in cleanup:
                pass
        return
    async for item in items:
        yield item
