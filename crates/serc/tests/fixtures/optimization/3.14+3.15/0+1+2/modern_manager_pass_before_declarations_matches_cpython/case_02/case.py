def f(manager):
    try:
        with manager as value:
            global marker
            pass
            global spare_marker
    finally:
        cleanup()
