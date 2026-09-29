async def f(task, clock, ready, sleep, suppress, Error, other_ready):
    try:
        deadline = clock() + 5.0
        while clock() < deadline:
            if ready():
                break
            if other_ready():
                break
            await sleep(0.01)
        else:
            raise AssertionError("timeout")
    finally:
        if not task.done():
            task.cancel()
            with suppress(Error):
                await task
    assert ready()
