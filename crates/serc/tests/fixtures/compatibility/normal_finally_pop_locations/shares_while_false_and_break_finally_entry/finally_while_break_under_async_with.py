async def f(manager, running):
    async with manager:
        try:
            while running():
                value = await read()
                if value == 'stop':
                    break
                await send(value)
        finally:
            await finish()
            close()
