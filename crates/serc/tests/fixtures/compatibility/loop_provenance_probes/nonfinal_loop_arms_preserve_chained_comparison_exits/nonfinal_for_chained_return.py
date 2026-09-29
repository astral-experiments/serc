def target(items, enabled, low, high):
    for item in items:
        if enabled:
            if low <= item < high:
                return item + 1
        else:
            if low <= item < high:
                return item
    return None
