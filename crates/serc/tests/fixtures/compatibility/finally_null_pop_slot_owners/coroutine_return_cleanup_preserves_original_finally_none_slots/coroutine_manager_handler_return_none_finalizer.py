from contextlib import suppress

async def f(manager, handle, flag):
    try:
        await work()
        with manager():
            if flag:
                return
            record(handle)
    except Exception as error:
        with manager():
            if flag:
                record(error)
                return
            report(error)
    finally:
        if handle is not None:
            handle.cancel()
            with suppress(Exception):
                await handle
