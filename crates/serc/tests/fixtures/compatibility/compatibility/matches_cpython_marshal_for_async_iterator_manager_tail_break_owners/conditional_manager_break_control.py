async def sample(outer, attempts, work, value, stop):
    with outer:
        try:
            async for attempt in attempts:
                with attempt:
                    result = await work(value)
                    if stop:
                        break
                    value.touch()
        except Exception:
            result = None
    first = value.field
    second = value.other
    return first, second, result
