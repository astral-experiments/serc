async def f(task, pending, progress):
    try:
        return await task()
    except ValueError as exc:
        return result(
            error=exc,
            link=None if progress is None else progress.link,
            response=None if progress is None else progress.response,
        )
    finally:
        if pending is not None:
            pending.cancel()
            await finish(pending)
