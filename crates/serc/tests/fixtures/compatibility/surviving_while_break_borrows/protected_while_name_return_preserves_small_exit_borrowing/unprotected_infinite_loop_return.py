def collect(factory):
    items = []
    reader = factory()
    while True:
        ready, item = reader.read()
        if ready:
            items.append(item)
        else:
            break
    return items
