def f(items, manager):
    with manager:
        for item in items:
            if check(item):
                consume(item)
            finish(item)
