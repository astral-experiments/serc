def f(manager, flag, items):
    try:
        with manager as value:
            pass
            assert flag
            work()
    finally:
        cleanup()
