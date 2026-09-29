def f(items):
    for index, item in enumerate(items):
        if item.kind == 0:
            if item.lines:
                for line in item.lines:
                    yield line
            elif 0 < index < len(items) - 1:
                yield None
        elif item.kind == 1:
            yield from item.lines
