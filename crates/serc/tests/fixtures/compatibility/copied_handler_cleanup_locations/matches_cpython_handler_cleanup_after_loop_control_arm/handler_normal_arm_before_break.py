def probe(items, parse, recover, choose, finish):
    for item in items:
        try:
            value = parse(item)
            break
        except KeyError:
            if choose(item):
                recover(item)
                finish(item)
            else:
                break
    return None
