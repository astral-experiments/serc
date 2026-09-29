async def collect(items, others, first, second, third, fourth, flag, limit, manager, keep, transform):
    async with manager:
        return [
            x
            for x in items
            if (
                not not (first(x) or second(x))
            )
        ]
