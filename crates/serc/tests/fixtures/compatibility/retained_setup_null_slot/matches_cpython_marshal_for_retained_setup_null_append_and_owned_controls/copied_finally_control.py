async def f(flag, first, second, timeout):
    try:
        if flag:
            return
    finally:
        with first:
            with second:
                try:
                    if timeout is None:
                        await run()
                except TimeoutError:
                    raise
                finally:
                    await close()
