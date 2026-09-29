def f(items, action, should_reraise, restore):
    try:
        values = [action(item) for item in items]
    except RuntimeError as error:
        if should_reraise(error):
            raise
        try:
            values = [action(item) for item in items]
        finally:
            raise RuntimeError
    result = [(index, sum(values)) for index in range(len(values))]
    return result, items
