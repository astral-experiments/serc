def f(manager):
    try:
        with manager as value:
            assert predicate()
            pass
            global marker
    finally:
        cleanup()
