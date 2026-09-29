def f(enabled, items, value, recover, after):
    if enabled:
        value = None
    else:
        try:
            for item in items:
                value = item
            assert value is not None
        except ValueError:
            recover()
    after(value)
