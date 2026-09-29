async def f(page, pager):
    return [item async for item in page], pager.token
