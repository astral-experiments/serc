
def f(items):
    for item in items:
        if condition(item):
            if item is not None:
                if item in (1, 2) or item in (3, 4):
                    assert item
