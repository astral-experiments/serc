def f(manager, other):
    try:
        with manager as value:
            pass
            assert predicate()
            pass
            global marker
    finally:
        cleanup()
    return value
