async def f(outer, inner, items, state):
    async with outer, inner:
        try:
            async for item in items:
                first()
                second()
                yield item
                return
        finally:
            if state.task is not None:
                await state.task
