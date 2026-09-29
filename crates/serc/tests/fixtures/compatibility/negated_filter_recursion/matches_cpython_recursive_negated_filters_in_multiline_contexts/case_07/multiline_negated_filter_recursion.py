async def collect(items, others, first, second, third, fourth, flag, limit, manager, keep, transform):
    return (
        x
        async for x in items
        if (
            not not x
        )
    )
