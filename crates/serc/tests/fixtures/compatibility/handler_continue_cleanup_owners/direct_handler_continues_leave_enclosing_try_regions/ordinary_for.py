def f(items):
    for item in items:
        try:
            try:
                work(item)
            except ValueError:
                continue
        except RuntimeError:
            recover()
