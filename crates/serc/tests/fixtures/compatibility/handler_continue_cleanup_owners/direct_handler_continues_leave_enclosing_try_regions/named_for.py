def f(items):
    for item in items:
        try:
            try:
                work(item)
            except ValueError as error:
                continue
        except RuntimeError:
            recover()
