
def f(items, first, second):
    for value in items:
        message = first + second
        if False:
            discard(value)
        else:
            assert digest(first) == digest(second), message
    return items
