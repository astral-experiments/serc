def f(manager, action, Error):
    while True:
        try:
            with manager:
                action()
                break
        except Error:
            pass
    return 0
