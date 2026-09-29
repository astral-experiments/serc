def f(ready, value, limit):
    try:
        if ready and 0 < value < limit:
            return True
        return False
    except Exception:
        return False
