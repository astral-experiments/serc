def f(flag):
    try:
        operation()
        assert flag
    except ValueError:
        raise
    finally:
        try:
            cleanup()
        except TypeError:
            pass
