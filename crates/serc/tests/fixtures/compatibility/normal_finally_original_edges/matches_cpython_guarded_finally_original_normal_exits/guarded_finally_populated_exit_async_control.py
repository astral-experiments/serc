async def collect(task, poll, finish):
    result = None
    try:
        await poll()
        if task.done():
            try:
                result = task.result()
            finally:
                task.pending = None
            finish(result)
    except TimeoutError:
        pass
    finish(result)
    return result
