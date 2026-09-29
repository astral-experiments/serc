def f(flag):
    try:
        operation()
        global declared
        assert flag
    except ValueError:
        raise
    finally:
        try:
            cleanup()
        except TypeError:
            pass
