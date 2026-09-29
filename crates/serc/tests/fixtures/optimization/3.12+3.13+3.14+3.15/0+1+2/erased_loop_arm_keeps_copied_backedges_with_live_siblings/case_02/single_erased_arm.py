def f(items, left, right, value):
    for item in items:
        if left or right:
            assert value
        else:
            consume(item)
