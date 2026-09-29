def f(manager, other):
    try:
        return work()
    except Exception as exc:
        with manager:
            with other:
                return fallback(exc)
