def f(items, copy):
    first, second, matrix = copy(items), copy(items), {key: {inner: 0 for inner in items} for key in items}
    return first, second, matrix
