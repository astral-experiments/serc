def f(items, choose, prepare, finish):
    result = None
    try:
        prepare()
    except ValueError:
        result = None
    else:
        for item in items:
            result = choose(item)
        else:
            result = tuple(items)
    finish(result)
    return result
