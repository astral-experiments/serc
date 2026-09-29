def outer(start, work, recover, cleanup, make):
    async def stream(*args, **kwargs):
        del args, kwargs
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
    return stream
