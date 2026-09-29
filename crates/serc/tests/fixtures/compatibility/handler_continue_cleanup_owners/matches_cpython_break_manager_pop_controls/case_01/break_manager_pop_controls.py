def f(manager, action):
    while True:
        with manager:
            action()
            break
    return 0
