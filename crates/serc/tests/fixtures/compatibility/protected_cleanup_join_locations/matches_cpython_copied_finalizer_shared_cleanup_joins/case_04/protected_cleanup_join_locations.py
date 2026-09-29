def f(value):
    try:
        try:
            return value
        finally:
            if value:
                raise ValueError
    finally:
        cleanup()
