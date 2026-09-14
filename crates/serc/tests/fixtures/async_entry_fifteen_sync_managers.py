async def probe(manager, factory, flag, use, finish):
    try:
        async with manager:
            with (
                factory(), factory(), factory(), factory(), factory(),
                factory(), factory(), factory(), factory(), factory(),
                factory(), factory(), factory(), factory(), factory(),
            ):
                if flag:
                    await use()
    finally:
        finish()
