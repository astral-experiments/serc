async def f(values, enabled):
    try:
        if enabled[0]:
            for value in values:
                await refresh(value)
                try:
                    consume(value)
                except LookupError:
                    break
    except RuntimeError:
        pass
    return None
