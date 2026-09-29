def f(manager):
    try:
        with manager as value:
            global marker
            pass
    finally:
        cleanup()
    return value
