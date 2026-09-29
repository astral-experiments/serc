def later_iterable(groups):
    return [
        value
        for group in groups
        for value in any(item for item in group)
    ]
