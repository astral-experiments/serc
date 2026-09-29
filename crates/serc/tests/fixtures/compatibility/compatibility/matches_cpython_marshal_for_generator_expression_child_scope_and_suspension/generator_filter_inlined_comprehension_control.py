def f(items, values):
    return (item for item in items if accept([inner for inner in values]))
