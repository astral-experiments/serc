def f(raw):
    try:
        value = int(raw)
        if 0 <= value <= 4:
            return value
        return None
    except Exception:
        return None
