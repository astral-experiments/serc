def f(items, a, b, c):
    for item in items:
        if a:
            if b:
                continue
            else:
                if not c:
                    continue
        else:
            sink(1)
            sink(2)
        finish(item)
