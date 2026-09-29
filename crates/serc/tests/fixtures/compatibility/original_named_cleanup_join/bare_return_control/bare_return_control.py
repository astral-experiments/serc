def f(manager):
    try:
        return work()
    except Exception:
        with manager:
            return fallback()
