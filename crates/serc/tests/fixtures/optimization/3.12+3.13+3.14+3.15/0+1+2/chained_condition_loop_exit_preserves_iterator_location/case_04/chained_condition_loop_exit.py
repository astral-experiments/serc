def f(a, b, items):
    if a < b < items:
        for item in items:
            consume(item)
        after()
