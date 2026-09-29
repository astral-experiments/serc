def f(next_item, values, consume):
    try:
        while True:
            item = next_item()
            if item is None:
                break
            consume(item)
    finally:
        for value in values:
            consume(value)
