async def f(flag, start, work, recover, cleanup, make):
    try:
        start()
        await work()
        if flag:
            yield make()
    except ValueError:
        recover()
        await work()
        raise
    finally:
        cleanup()
