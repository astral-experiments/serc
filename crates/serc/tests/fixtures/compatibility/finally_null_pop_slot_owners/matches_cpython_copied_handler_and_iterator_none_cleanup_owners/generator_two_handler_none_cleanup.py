def f(value):
    try:
        try:
            operation()
        except LookupError:
            try:
                other()
            except KeyError:
                return
        yield value
    finally:
        if value is not None:
            cleanup(value)
