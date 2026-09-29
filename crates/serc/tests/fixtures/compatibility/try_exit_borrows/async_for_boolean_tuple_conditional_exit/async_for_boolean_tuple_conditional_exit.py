async def f(items):
    status = None
    domain = None
    found = False
    async for item in items:
        if item.skip:
            if item.stop:
                break
            continue
        if status is None:
            status = await item.label()
        if not found and item.accepted:
            found = True
            domain = await item.domain()
        if status is not None and found:
            break
    return status or "", domain
