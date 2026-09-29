def f(items, manager):
    with manager:
        for item in items:
            flag = check(item)
            if flag:
                consume(item)
