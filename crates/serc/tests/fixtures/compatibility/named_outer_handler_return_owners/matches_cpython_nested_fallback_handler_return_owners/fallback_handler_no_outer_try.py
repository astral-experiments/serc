def f():
    try:
        return primary()
    except ValueError:
        try:
            return fallback()
        except LookupError:
            pass
