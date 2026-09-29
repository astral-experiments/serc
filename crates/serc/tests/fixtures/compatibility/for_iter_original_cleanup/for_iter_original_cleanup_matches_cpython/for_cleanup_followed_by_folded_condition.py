def f(items, recover):
    for item in items:
        try:
            return item
        except Exception:
            recover()
    if False:
        recover()
    return f'{items}'
