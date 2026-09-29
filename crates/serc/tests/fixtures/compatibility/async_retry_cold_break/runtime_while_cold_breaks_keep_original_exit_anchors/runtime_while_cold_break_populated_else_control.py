def f(running, poll, stopped, finish, Retry):
    while running():
        try:
            poll()
            break
        except Retry:
            pass
        if stopped():
            break
    else:
        finish()
    result = finish()
    return result
