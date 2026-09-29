async def f(flag, first, second, timeout):
    if flag:
        prepare()
    with first:
        with second:
            try:
                if timeout is None:
                    await run()
                else:
                    await other()
            except TimeoutError:
                raise
            finally:
                await close()
