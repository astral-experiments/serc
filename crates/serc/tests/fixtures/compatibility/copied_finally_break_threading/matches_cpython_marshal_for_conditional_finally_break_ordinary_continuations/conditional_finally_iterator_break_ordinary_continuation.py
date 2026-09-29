def process(items, acquire, choose, finish):
    for item in items:
        reader = None
        try:
            reader = acquire(item)
            if choose(item):
                break
        finally:
            if reader is not None:
                reader.close()
    finish(reader)
    return reader
