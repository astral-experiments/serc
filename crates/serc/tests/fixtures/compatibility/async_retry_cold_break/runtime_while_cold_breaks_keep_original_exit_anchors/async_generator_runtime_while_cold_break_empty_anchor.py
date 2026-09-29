async def f(running, poll, stopped, finish, Retry):
    while running():
        try:
            yield await poll()
            break
        except Retry:
            pass
        if stopped():
            break
    result = finish()
    yield result
