def f(work, consume, value):
    try:
        return work()
    except TimeoutError:
        try:
            work()
        except LookupError:
            pass
        except PermissionError:
            try:
                work()
            except (ValueError, OSError):
                return (124, value or "", consume(value))
        return (1, value, consume(value))
