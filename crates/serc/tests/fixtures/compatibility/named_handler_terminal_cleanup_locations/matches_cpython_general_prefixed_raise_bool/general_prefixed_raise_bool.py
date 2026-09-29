async def probe(action, lookup, first):
    try:
        await action()
    except Exception as error:
        current = await lookup()
        if current and first:
            raise RuntimeError() from error
