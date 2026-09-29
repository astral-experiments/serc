def f(items):
    for item in items:
        if item:
            assert item
        else:
            assert items
        value = item
    yield value
