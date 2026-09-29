def collect(items, others, first, second, third, fourth, flag, limit, manager, keep, transform):
    with manager:
        return [
            x
            for x in items
            if (
                not (first(x) or second(x))
            )
        ]
