def select(items, release):
    for item in items:
        try:
            if item == "stop":
                break
            if item == "skip":
                continue
            if item:
                return item, "found"
        finally:
            release()
    return None, "missing"
