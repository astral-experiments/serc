async def f(pager):
    async for page in pager:
        return [item async for item in page], pager.token
    return [], None
