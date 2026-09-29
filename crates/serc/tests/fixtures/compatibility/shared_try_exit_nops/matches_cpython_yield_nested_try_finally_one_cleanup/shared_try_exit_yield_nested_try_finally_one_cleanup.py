def f(value):
    try:
        yield value
        try:
            ping()
        except Exception:
            pass
    finally:
        try:
            cleanup()
        except Exception:
            pass
