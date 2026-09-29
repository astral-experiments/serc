def probe(items, parse, recover, keep, finish):
    for item in items:
        try:
            value = parse(item)
        except ValueError:
            if not keep(item):
                continue
            else:
                value = recover(item)
        finish(value)
    return None
