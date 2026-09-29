async def run(manager, cleanup):
    try:
        async with manager:
            await cleanup()
    except Exception as error:
        raise RuntimeError(error) from error
