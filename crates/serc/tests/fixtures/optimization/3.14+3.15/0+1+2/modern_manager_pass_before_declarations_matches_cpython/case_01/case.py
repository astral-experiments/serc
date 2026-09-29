def f(manager):
    try:
        with manager as value:
            pass
            global marker
    finally:
        cleanup()
