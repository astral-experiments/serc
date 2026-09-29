def captured_target(items, predicate):
    return [
        (left, right)
        for left, right in items
        if all(predicate(right, value) for value in right)
    ]
