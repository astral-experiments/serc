async def f(items, status, domain):
    async for item in items:
        if item.skip:
            continue
        if item.stop:
            break
    return status or "", domain
