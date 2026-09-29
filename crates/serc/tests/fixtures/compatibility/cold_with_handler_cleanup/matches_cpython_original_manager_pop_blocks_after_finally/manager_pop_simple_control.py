def f(manager):
    with manager:
        try:
            work()
        finally:
            close()
