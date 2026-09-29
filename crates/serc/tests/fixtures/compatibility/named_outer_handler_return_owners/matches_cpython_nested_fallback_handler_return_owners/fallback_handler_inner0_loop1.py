def f(items, ready):
    for item in items:
        try:
            if ready:
                try:
                    return primary()
                except ValueError:
                    return fallback()
            finish()
        except Exception:
            recover()
