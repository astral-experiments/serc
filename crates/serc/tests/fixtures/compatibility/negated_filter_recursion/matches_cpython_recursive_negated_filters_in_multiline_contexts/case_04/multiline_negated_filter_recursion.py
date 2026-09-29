def collect(items, others, first, second, third, fourth, flag, limit, manager, keep, transform):
    try:
        observe()
    except Exception as error:
        return [
            x
            for x in items
            if (
                not (first(x) and not (second(x) or third(x)))
            )
        ]
