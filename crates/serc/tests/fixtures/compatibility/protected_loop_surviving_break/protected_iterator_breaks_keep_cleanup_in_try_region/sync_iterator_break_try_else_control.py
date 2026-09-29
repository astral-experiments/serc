async def run(items, ready, recover):
    try:
        for item in items:
            if ready(item):
                break
            yield item
    except ValueError as error:
        await recover(error)
    else:
        await recover(None)
    yield None
