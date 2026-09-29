def f(items, action, should_reraise, restore):
    try:
        values = [action(item) for item in items]
    except RuntimeError as error:
        try:
            values = [action(item) for item in items]
        finally:
            restore()
    result = [(index, sum(values)) for index in range(len(values))]
    return result, items
