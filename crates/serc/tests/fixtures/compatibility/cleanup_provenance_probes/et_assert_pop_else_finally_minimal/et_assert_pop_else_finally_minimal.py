def f(value):
    try:
        try:
            assert value
        except Error:
            raise
        else:
            done(value)
    finally:
        cleanup()
