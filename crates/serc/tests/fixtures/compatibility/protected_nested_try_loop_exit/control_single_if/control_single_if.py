def f(items, inner, consume, recover):
    for item in items:
        try:
            if inner(item):
                consume(item)
        except Exception:
            recover(item)
