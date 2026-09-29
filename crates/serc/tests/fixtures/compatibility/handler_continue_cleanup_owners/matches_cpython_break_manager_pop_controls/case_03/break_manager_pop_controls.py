def f(manager, action, Error):
    while True:
        with manager:
            try:
                action()
            except Error:
                break
    return 0
