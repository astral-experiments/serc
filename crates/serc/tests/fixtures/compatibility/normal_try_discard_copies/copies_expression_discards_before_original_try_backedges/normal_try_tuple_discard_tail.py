def f(items):
    for item in items:
        try:
            tuple(int(part) for part in item.split('.'))
        except ValueError:
            pass
