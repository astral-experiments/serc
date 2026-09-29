async def f(flag, client):
    previous = None
    options = {'enabled': True}
    try:
        if flag:
            await client.update(options)
        else:
            await client.create(options)
    except Exception:
        await client.rollback(previous)
        raise
    return options
