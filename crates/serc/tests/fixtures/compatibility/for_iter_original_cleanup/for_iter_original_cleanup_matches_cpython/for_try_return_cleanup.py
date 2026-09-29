def f(items, recover):
    for item in items:
        try:
            return item
        except Exception:
            recover()
    raise RuntimeError(f'{items}')
