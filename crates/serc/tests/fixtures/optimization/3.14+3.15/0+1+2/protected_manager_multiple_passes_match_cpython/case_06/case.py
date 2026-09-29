def f(manager, other):
    try:
        with manager as value:
            pass
            pass
            pass
    finally:
        cleanup()
    return value
