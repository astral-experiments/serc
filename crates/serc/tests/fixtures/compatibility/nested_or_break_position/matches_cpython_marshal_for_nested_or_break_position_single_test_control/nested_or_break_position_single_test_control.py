def collect(source, timeout):
    items = []
    value = None
    while True:
        item = read_next(source, timeout)
        if not item:
            break
        items.append(item)
        if len(item) >= 3 and item[:3].isdigit():
            value = int(item[:3])
            if item[3] != '-':
                break
        else:
            break
        if len(items) >= 100:
            break
    return value, items
