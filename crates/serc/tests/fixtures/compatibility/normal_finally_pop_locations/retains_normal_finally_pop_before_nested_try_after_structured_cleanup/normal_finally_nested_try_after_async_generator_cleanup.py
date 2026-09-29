async def target(stream, cleanup, finish):
    failure = None
    try:
        try:
            async for item in stream:
                yield item
        finally:
            cleanup()
    except BaseException as exc:
        failure = exc
        raise
    finally:
        try:
            finish(failure)
        except Exception:
            pass
