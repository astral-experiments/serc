async def f(items, parse, log, mode, update):
    for item in items:
        try:
            result = parse(item)
        except ValueError:
            log(item)
        else:
            if mode == 1:
                update(result, 1)
            elif mode == 2:
                update(result, 2)
        if item is not None:
            log(item)
    return items
