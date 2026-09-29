def f(ready):
    try:
        if ready:
            try:
                return primary()
            except ValueError:
                return fallback()
        finish()
    except Exception:
        recover()
