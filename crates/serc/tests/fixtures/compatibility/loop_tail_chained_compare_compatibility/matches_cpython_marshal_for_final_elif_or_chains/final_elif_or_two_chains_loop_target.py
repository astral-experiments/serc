def count(items, counts):
    for value in items:
        if value is None:
            counts["missing"] += 1
        elif 0 <= value < 10 or 20 <= value < 30:
            counts["range"] += 1
    return counts
