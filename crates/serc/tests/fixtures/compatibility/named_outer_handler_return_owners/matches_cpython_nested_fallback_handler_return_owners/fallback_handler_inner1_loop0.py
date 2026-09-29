def f(ready):
    try:
        if ready:
            try:
                return primary()
            except ValueError:
                try:
                    return fallback()
                except LookupError:
                    pass
        finish()
    except Exception:
        recover()
