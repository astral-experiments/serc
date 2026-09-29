def probe(items, parse, recover, keep, finish):
    for item in items:
        try:
            value = parse(item)
        except ValueError:
            if keep(item):
                value = recover(item)
            else:
                continue
        finish(value)
    return None
