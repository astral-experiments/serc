async def recovered(items, first, second):
    try:
        async for item in items:
            break
    except Exception:
        first = True
    finally:
        await cleanup()
    if first:
        assert first
    if second:
        assert second
    else:
        assert first
