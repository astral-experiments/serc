def f(value, make, touch, cleanup):
    result = make(value)
    try:
        touch(result)
    except Exception:
        cleanup(result)
        raise
    return result
