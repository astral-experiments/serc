def collect(factory):
    items = []
    try:
        reader = factory()
        while True:
            ready, item = reader.read()
            if ready:
                items.append(item)
            else:
                break
        return items
    except Exception as error:
        pass
