def consume(manager, items, select, read, convert):
    value = None
    with manager():
        for item in items:
            if select(item):
                try:
                    value = read(item)
                except ValueError:
                    pair = read(None)
                    if len(pair) == 2:
                        value = (convert(pair[0]), convert(pair[1]))
            else:
                value = item
    return value
