async def f(session, cleanup, work):
    try:
        try:
            return
        finally:
            if session is not None:
                await cleanup(session)
                session = None
    finally:
        await work()
