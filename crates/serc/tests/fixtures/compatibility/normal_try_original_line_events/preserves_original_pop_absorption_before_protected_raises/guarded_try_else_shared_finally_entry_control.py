def inspect_checked(acquire, cleanup, expected):
    try:
        prepare()
        try:
            value = acquire()
        except OSError:
            value = expected
        else:
            if value != expected:
                raise RuntimeError('changed')
    finally:
        if expected is not None:
            cleanup(expected)
    return value
