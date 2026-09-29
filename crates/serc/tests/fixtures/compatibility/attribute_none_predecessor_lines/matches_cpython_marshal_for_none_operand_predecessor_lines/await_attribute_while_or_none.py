async def f(fetch, other, pause, manager):
    async with manager:
        while (await fetch())[
            0
        ].value is None or other()[
            0
        ].value is None:
            await pause()
