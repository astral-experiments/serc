def f(manager, other):
    try:
        with manager:
            pass
            pass
            global marker
    finally:
        cleanup()
    return 42
