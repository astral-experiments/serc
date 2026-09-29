def f(value, other, manager, flag, items, consume, poll):
    with manager:
        try:
            while poll() is None:
                consume(value)
                consume(other)
        except Error:
            try:
                consume(other)
            except InnerError:
                consume(value)
            try:
                consume(other)
            except InnerError:
                consume(other)
    return value, other
