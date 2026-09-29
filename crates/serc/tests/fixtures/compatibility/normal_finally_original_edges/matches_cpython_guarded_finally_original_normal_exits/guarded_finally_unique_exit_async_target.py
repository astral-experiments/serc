async def collect(task, poll, finish):
    result = None
    try:
        await poll()
        if task.done():
            try:
                result = task.result()
            finally:
                task.pending = None
    except TimeoutError:
        pass
    finish(result)
    return result
