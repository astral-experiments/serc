def select(items):
    for item in items:
        if item == "stop":
            break
        if item == "skip":
            continue
        if item:
            return item, "found"
    return None, "missing", False
