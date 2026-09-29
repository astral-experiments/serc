def f(items, values):
    return (result for item in items for result in expand(item, [inner for inner in values]))
