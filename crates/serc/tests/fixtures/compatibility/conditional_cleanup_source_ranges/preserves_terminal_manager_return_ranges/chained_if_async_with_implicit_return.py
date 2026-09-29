async def inspect(low, value, high, manager, emit):
    if low <= value < high:
        async with manager():
            await emit(value)
