def collect_values(enabled, direct, alternate, items, convert):
    first = None
    result = []
    if enabled:
        if direct:
            assert items
            result = [convert(items[0], flag=True)]
        else:
            result = [
                convert(
                    first,
                    flag=False,
                )
            ]
    elif alternate:
        result = [first]
    return first, result
