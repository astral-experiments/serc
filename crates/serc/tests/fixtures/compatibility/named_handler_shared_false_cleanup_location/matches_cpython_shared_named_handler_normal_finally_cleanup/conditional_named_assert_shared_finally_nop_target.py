def f(work, first, second, enabled, cleanup):
    try:
        try:
            work()
        except RuntimeError as error:
            assert first(error)
            if enabled:
                assert second(error)
        else:
            raise AssertionError()
    finally:
        cleanup()
