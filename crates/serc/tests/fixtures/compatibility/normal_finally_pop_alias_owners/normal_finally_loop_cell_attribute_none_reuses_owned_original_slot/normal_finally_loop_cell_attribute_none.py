def outer(state):
    async def f(manager, active, work, cleanup):
        async with manager:
            try:
                while active:
                    await work()
            finally:
                if state.resource is not None:
                    cleanup()
    return f
