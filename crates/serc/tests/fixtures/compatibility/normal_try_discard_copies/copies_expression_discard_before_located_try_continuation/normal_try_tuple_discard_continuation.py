def f(items):
    filtered = []
    for item in items:
        try:
            tuple(int(part) for part in item.split('.'))
        except ValueError:
            continue
        filtered.append(item)
    return filtered
