def f(items, flag, left, right, after):
    for item in items:
        try:
            left(item) if flag else right(item)
        except ValueError:
            continue
        after(item)
