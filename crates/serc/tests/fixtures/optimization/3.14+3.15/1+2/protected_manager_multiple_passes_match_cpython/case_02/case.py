def f(manager, other):
    try:
        with manager:
            pass
            assert predicate()
            pass
            global marker
    finally:
        cleanup()
    return 42
