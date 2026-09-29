def f(manager):
    try:
        with manager as value:
            pass
            assert predicate()
            global marker
    finally:
        cleanup()
