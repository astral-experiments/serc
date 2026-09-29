def f(value):
    try:
        try:
            return
        finally:
            if value is not None:
                value = None
            done()
    finally:
        cleanup()
