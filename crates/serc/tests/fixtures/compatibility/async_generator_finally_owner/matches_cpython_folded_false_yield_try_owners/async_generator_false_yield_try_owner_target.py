async def f(start, work, recover, cleanup, make):
    try:
        start()
        await work()
        if False:
            yield make()
    except ValueError:
        recover()
        await work()
        raise
    finally:
        cleanup()
