def collect_values(enabled, direct, alternate, items, convert):
    first = None
    result = []
    if enabled:
        result = [
            convert(
                item,
                flag=False,
            )
            for item in items
        ]
    elif alternate:
        result = [first]
    return first, result
