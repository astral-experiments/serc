async def sample(outer, attempts, work, value):
    with outer:
        try:
            async for attempt in attempts:
                with attempt:
                    result = await work(value)
                    break
        except Exception:
            result = None
    first = value.field
    second = value.other
    return first, second, result
