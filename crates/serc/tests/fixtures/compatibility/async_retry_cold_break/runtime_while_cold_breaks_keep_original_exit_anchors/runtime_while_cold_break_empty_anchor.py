def f(running, poll, stopped, finish, Retry):
    while running():
        try:
            poll()
            break
        except Retry:
            pass
        if stopped():
            break
    result = finish()
    return result
