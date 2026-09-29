def f(items, strict, silent, call):
    for item in items:
        try:
            call(item)
        except Exception:
            if strict:
                raise
            elif not silent:
                call(item)
