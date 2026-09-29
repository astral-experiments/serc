def f(items, flag, left, right):
    for item in items:
        try:
            left(item) if flag else right(item)
        except ValueError:
            pass
