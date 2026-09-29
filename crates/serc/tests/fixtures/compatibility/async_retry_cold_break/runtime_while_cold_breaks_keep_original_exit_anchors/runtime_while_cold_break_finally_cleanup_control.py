def f(running, poll, stopped, cleanup, finish, Retry):
    while running():
        try:
            poll()
            break
        except Retry:
            pass
        if stopped():
            try:
                break
            finally:
                cleanup()
    result = finish()
    return result
