async def collect(items, manager, selected):
    with manager:
        if selected:
            prepare()
        else:
            async for item in items:
                await consume(item)
