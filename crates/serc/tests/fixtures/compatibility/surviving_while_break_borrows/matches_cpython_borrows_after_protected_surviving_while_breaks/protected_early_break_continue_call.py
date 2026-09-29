def collect(reader):
    items = []
    try:
        while True:
            ready, item = reader.read()
            if not ready:
                if reader.done():
                    break
                continue
            items.append(item)
            if reader.done():
                break
        count = reader.finish(items)
    finally:
        reader.close()
    return count
