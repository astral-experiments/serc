def loops(items):
    total = 0
    for index, item in enumerate(items):
        if item is None:
            continue
        total += item

    for index in range(200):
        if index > 100:
            break
        total += index

    remaining = len(items)
    while remaining:
        remaining -= 1
        if remaining == 2:
            continue
        total += remaining

    return total
