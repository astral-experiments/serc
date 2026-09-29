def f(work, consume, value):
    try:
        return work()
    except TimeoutError as outer:
        try:
            work()
        except (ValueError, OSError):
            return (124, value or "", consume(outer))
        return (1, value, consume(outer))
