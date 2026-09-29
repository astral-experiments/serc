def f(manager, other):
    try:
        with manager as value:
            pass
            global marker
            pass
            global spare_marker
    finally:
        cleanup()
    return value
