def target(items, enabled, low, high, consume):
    for item in items:
        if enabled:
            if low <= item < high:
                return item
        else:
            consume(item)
    return False
