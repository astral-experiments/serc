def f(manager, flag, items):
    try:
        with manager:
            pass
            work()
    finally:
        cleanup()
