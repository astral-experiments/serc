def probe(items, parse, recover, choose, finish):
    for item in items:
        try:
            value = parse(item)
            break
        except KeyError:
            if not choose(item):
                break
            else:
                recover(item)
                finish(item)
    return None
