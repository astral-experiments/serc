async def target(enabled, items):
    if enabled:
        try:
            yield make_value()
        finally:
            async for item in items:
                consume(item)
            else:
                finish()
        return
    yield None
