def f(manager):
    try:
        return work()
    except Exception as exc:
        with manager:
            return fallback(exc)
