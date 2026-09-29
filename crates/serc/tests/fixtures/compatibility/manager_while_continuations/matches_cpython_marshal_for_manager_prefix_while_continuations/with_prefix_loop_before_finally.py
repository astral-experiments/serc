def f(manager, ready, work, cleanup):
    with manager:
        while not ready():
            work()
        try:
            work()
        finally:
            cleanup()
