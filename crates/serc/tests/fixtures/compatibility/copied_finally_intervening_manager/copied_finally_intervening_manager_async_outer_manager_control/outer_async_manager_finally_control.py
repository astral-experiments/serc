async def run(manager, value, release, finish):
    async with manager:
        try:
            try:
                return await value()
            finally:
                await release()
        finally:
            await finish()
