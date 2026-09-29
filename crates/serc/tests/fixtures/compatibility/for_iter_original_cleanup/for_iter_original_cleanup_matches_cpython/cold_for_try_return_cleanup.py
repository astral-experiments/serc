def f(items, run, recover):
    try:
        run()
    except Exception:
        for item in items:
            try:
                return item
            except ValueError:
                recover()
        raise RuntimeError(f'{items}')
