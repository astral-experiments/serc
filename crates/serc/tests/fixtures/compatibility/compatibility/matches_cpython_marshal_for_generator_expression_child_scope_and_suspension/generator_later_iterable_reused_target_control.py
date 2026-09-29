def f(items, values):
    return (result for item in items for result in expand([item for item in values]))
