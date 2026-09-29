async def run(manager, value, release, finish):
    try:
        async with manager:
            try:
                return await value()
            finally:
                await release()
    finally:
        await finish()
