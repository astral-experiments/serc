def f(work, first, second, cleanup):
    try:
        try:
            work()
        except RuntimeError as error:
            assert first(error)
            assert second(error)
        else:
            raise AssertionError()
    finally:
        cleanup()
