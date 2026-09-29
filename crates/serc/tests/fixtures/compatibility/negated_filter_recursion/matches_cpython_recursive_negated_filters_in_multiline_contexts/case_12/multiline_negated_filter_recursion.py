def collect(items, others, first, second, third, fourth, flag, limit, manager, keep, transform):
    try:
        observe()
    except* Exception as error:
        result = (
            x
            for x in items
            if (
                not (first(x) or not second(x))
            )
        )
    return result
