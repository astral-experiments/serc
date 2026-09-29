def f(manager):
    try:
        with manager:
            global marker
    finally:
        cleanup()
