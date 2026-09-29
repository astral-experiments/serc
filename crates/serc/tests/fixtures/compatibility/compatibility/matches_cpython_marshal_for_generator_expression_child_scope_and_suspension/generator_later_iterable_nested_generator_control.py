def f(items, values):
    return (result for item in items for result in (inner for inner in values))
