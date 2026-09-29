def run(exists, write, read, cache, items, consume, force):
    if not exists(cache) or force:
        values = []
        for item in items:
            values.append(item)
        write([(item,) for item in values], cache)
    else:
        values = [item for (item,) in read(cache)]
    result = consume(values)
    consume(cache, result, values)
