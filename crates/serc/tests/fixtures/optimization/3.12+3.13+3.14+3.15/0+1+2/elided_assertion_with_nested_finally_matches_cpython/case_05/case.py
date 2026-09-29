def f():
    try:
        operation()
        global g
    except ValueError:
        raise
    finally:
        try:
            cleanup()
        except TypeError:
            pass
