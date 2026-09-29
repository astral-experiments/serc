def collect(items, others, first, second, third, fourth, flag, limit, manager, keep, transform):
    return [
        x
        for x in items
        for y in others
        if keep(y)
        if (
            not not (first(x) or second(x))
        )
    ]
