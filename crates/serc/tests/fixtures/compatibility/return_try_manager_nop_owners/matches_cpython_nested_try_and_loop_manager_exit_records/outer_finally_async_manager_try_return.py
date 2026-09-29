async def f(manager, active):
    try:
        async with manager:
            while active:
                await tick()
                try:
                    if ready():
                        record()
                        return
                except ValueError:
                    recover()
    finally:
        if active:
            await cleanup()
