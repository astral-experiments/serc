async def f(flag, first, second, obj):
    if flag:
        prepare()
    with first:
        with second:
            try:
                if obj.timeout is None:
                    await run()
                else:
                    await other()
            except TimeoutError:
                raise
            finally:
                await close()
