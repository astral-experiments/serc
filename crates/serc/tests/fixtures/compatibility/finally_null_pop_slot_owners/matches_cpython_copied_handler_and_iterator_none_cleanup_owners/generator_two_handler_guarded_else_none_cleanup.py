def f(value, expected):
    try:
        try:
            operation()
        except LookupError:
            try:
                other()
            except KeyError:
                return
        try:
            yield value
        except BaseException:
            raise
        else:
            if value != expected:
                raise RuntimeError('changed')
    finally:
        if value is not None:
            cleanup(value)
