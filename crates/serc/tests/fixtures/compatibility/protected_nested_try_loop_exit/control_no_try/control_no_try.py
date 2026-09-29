def f(items, outer, inner, consume):
    for item in items:
        if outer(item):
            if inner(item):
                consume(item)
