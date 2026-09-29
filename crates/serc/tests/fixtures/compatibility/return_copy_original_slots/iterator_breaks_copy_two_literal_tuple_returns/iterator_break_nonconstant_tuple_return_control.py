def select(items, fallback):
    for item in items:
        if item == "stop":
            break
        if item == "skip":
            continue
        if item:
            return item, "found"
    return fallback, "missing"
