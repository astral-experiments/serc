def f(value, make, touch, cleanup):
    try:
        result = make(value)
        try:
            touch(result)
        except Exception:
            cleanup(result)
            raise
        return result
    finally:
        pass
