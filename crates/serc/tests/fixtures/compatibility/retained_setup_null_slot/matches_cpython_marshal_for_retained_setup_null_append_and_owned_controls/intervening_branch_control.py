async def f(flag, first, second, timeout):
    with first:
        with second:
            if flag:
                prepare()
            try:
                if timeout is None:
                    await run()
            except TimeoutError:
                raise
            finally:
                await close()
