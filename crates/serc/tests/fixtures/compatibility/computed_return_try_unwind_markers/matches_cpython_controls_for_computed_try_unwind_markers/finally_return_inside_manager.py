def f(manager, value):
    with manager:
        try:
            return value
        finally:
            cleanup()
