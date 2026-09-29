async def collect(items, others, first, second, third, fourth, flag, limit, manager, keep, transform):
    return [
        x
        for x in items
        if (
            not (first(x) or (second(x) and third(x)))
        )
    ]
