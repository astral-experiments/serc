def f(manager):
    with manager:
        pass
        try:
            work()
        finally:
            cleanup()
