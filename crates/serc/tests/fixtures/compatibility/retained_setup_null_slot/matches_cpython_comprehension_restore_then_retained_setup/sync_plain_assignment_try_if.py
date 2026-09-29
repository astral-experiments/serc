def f(value):
    ready = value
    try:
        if ready:
            return consume(value)
    except Exception:
        return None
    return value
