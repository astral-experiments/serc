def f(value):
    try:
        try:
            operation()
        except LookupError:
            return
        yield value
    finally:
        if value is not None:
            cleanup(value)
