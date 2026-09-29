def count(items, counts):
    for value in items:
        if value is None:
            counts["missing"] += 1
        elif value != 5 and 0 <= value < 10:
            counts["range"] += 1
    return counts
