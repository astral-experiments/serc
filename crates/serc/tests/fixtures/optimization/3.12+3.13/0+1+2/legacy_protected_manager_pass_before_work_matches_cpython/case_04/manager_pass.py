def f(manager, flag, items):
    try:
        with manager:
            pass; pass
            raise ValueError()
    finally:
        cleanup()
