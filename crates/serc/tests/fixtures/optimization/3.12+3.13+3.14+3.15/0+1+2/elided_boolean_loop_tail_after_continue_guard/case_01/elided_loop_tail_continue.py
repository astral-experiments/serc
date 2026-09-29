
def f(items, a, b, c, d):
    for item in items:
        if item <= 0:
            continue
        if a != b or c != d:
            assert item
