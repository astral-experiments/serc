def f(items, exact, minimum, maximum, limit, variable):
    for item in items:
        if first(item):
            consume(item)
        elif not exact and minimum == 0 and 0 < maximum < limit and not variable:
            consume(item)
