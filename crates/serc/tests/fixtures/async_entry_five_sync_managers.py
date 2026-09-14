async def probe(outer, inner, factory, flag, use):
    async with outer:
        await prepare()
        async with inner:
            with (
                factory(), factory(), factory(), factory(), factory(),
            ):
                if flag:
                    await use()
