async def f(items, prepare, recover, finish):
    result = None
    try:
        await prepare()
    except ValueError as exc:
        result = str(exc)
    else:
        for item in items:
            result = await recover(item)
            if result:
                break
    await finish(result)
    return result
