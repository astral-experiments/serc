def f(items):
    for item in items:
        try:
            work(item)
        except ValueError:
            continue
