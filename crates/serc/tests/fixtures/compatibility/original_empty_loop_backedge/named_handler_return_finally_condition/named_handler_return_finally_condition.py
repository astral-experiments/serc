async def f(task, pending):
    try:
        return await task()
    except ValueError as exc:
        return result(error=exc)
    finally:
        if pending is not None:
            pending.cancel()
            await finish(pending)
