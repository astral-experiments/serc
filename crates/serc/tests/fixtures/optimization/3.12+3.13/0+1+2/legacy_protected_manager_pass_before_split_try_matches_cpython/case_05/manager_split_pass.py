def f(manager):
    try:
        return work()
    finally:
        with manager:
            pass
            try:
                work()
            finally:
                cleanup()
