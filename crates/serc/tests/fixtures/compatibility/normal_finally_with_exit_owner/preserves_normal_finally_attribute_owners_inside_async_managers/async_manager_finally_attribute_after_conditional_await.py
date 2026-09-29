async def f(self, session, run, hook, cleanup):
    try:
        async with session:
            error = None
            try:
                result = await run()
                if hook is not None:
                    await hook()
            except BaseException as caught:
                error = caught
                raise
            finally:
                if self.enabled:
                    try:
                        await session.stop()
                    except Exception:
                        if error is None:
                            raise
                        self.record(error)
    finally:
        await cleanup(session)
    return result
