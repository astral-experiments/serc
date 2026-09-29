
def f(items, emit):
    for item in items:
        remaining = item
        if 0:
            discard(item)
        else:
            while remaining > 0:
                emit(item, remaining)
                remaining -= 1
    return items
