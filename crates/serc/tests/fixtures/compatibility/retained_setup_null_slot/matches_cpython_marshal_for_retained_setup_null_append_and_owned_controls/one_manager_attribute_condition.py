async def f(flag, first, obj):
    if flag:
        prepare()
    with first:
        try:
            if obj.timeout is None:
                await run()
            else:
                await other()
        except TimeoutError:
            raise
        finally:
            await close()
