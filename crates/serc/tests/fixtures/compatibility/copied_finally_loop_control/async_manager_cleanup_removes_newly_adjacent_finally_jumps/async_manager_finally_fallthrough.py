async def f(manager, reader, pending, other):
    try:
        async with manager:
            try:
                while True:
                    value = await read()
                    if value is None:
                        break
                    yield value
            finally:
                reader.cancel()
                with suppress(Exception):
                    await reader
                if pending is not None:
                    pending.cancel()
                    with suppress(CancelledError):
                        await pending
                if other is not None and not other.done():
                    other.cancel()
                    with suppress(CancelledError):
                        await other
    except Exception as error:
        recover(error)
        try:
            if pending is not None:
                pending.cancel()
                with suppress(CancelledError):
                    await pending
            if other is not None and not other.done():
                other.cancel()
                with suppress(CancelledError):
                    await other
        except Exception:
            pass
    finally:
        try:
            finish()
        except Exception:
            pass
