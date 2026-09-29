def outer():
    saved = None
    def target(active, inner, choose, value, left, right):
        nonlocal saved
        if active:
            try:
                if inner:
                    left(value) if choose else right(value)
            except Exception:
                left(value) if choose else right(value)
        else:
            left(value) if choose else right(value)
        saved = value
    return target
