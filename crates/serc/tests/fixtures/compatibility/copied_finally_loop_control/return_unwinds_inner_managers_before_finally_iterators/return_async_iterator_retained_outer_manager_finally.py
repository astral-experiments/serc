async def f(items, outer_manager, inner_manager, value):
    with outer_manager:
        try:
            async for item in items:
                with inner_manager:
                    return item, value
        finally:
            try:
                await finish()
            except Exception:
                report()
