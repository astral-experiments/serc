def f(manager, other):
    try:
        with manager as value:
            pass
            global marker
            pass
    finally:
        cleanup()
    return value
