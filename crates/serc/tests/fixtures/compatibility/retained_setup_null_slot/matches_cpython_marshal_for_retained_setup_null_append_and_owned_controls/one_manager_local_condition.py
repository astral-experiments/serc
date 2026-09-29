async def f(flag, first, timeout):
    if flag:
        prepare()
    with first:
        try:
            if timeout is None:
                await run()
            else:
                await other()
        except TimeoutError:
            raise
        finally:
            await close()
