async def f(task):
    result = None
    cancelled = None
    try:
        result = await shield(task)
    except OuterError as exc:
        cancelled = exc
        while not task.done():
            try:
                await wait(task)
            except InnerError as repeat:
                cancelled = repeat
        result = task.result()
    if cancelled is not None:
        notify(cancelled)
    return consume(result)
