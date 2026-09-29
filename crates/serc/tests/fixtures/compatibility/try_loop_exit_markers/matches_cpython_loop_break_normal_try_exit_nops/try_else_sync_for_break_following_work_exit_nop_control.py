def f(items, choose, prepare, finish):
    result = None
    try:
        prepare()
    except ValueError:
        result = None
    else:
        for item in items:
            if choose(item):
                result = item
                break
        else:
            result = tuple(items)
        finish(result)
    finish(result)
    return result
