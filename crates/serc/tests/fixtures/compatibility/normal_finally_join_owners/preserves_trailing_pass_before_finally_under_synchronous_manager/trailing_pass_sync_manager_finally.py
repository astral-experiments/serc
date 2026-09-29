def f(manager, work, close):
    with manager:
        try:
            work()
            pass
        finally:
            close()
