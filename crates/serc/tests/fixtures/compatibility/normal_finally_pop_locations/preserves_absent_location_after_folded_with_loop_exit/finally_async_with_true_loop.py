async def f(manager):
    try:
        async with manager:
            while True:
                await step()
    finally:
        await finish()
