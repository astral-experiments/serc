async def f(manager, running):
    try:
        async with manager:
            while running():
                await step()
    finally:
        await finish()
