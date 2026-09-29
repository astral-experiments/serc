def f(items, recover):
    for item in items:
        try:
            return item
        except Exception:
            recover()
    else:
        return f'{items}'
