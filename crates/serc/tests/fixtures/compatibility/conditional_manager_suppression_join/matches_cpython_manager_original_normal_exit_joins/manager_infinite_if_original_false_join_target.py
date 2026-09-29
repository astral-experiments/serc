def f(manager, enabled, work):
    with manager():
        work()
        if enabled:
            while True:
                work()
