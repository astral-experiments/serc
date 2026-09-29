def f(flag):
    try:
        operation()
    except ValueError:
        raise
    finally:
        try:
            cleanup()
        except TypeError:
            pass
