
def f(groups, a, b, c, d):
    for items in groups:
        for item in items:
            if item is None:
                continue
            value = transform(item)
            if a != b or c != d:
                assert value
        action()
