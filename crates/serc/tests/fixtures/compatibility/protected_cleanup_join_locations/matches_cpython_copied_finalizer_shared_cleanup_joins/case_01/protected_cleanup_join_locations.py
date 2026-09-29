async def f(manager, outer, session, active, cleanup):
    with manager:
        try:
            async with outer:
                try:
                    while active:
                        try:
                            await work()
                        except Exception:
                            return
                        finally:
                            if session:
                                await close(session)
                                session = None
                finally:
                    finish()
        finally:
            done()
