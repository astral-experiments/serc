def f(items, left):
    return [item for item in items
            if left is item is None]
