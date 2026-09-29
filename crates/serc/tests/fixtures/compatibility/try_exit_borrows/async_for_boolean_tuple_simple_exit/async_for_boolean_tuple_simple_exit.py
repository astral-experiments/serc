async def f(items, status, domain):
    async for item in items:
        if item:
            break
    return status or "", domain
