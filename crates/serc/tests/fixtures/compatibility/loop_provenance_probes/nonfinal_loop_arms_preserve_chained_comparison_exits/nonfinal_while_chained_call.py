def target(read, enabled, low, high, consume):
    while True:
        item = read()
        if enabled:
            if low <= item < high:
                consume(item)
        else:
            consume(None)
