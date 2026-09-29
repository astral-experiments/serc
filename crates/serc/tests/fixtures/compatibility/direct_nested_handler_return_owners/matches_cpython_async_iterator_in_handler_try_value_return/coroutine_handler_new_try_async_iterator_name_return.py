async def probe(events, operation, result):
    try:
        try:
            await operation()
        except ValueError:
            try:
                async for item in events:
                    if item:
                        return result
            except TypeError:
                raise
    except RuntimeError:
        pass
    return result
