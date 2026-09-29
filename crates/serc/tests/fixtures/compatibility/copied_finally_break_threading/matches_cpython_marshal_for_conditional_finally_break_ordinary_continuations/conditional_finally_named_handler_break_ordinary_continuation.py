def process(items, acquire, choose, report, finish):
    reader = None
    for item in items:
        try:
            reader = acquire(item)
            if choose(item):
                break
        except Exception as error:
            report(error)
            break
        finally:
            if reader is not None:
                reader.close()
    finish(reader)
    return reader
