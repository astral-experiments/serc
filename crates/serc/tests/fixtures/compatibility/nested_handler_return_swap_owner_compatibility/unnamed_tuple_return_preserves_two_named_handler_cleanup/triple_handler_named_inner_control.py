def f(work, consume, value):
    try:
        return work()
    except TimeoutError as outer:
        try:
            work()
        except LookupError:
            pass
        except PermissionError as middle:
            try:
                work()
            except (ValueError, OSError) as inner:
                return (124, value or "", consume(f"{outer} {middle} {inner}"))
        return (1, value, consume(outer))
