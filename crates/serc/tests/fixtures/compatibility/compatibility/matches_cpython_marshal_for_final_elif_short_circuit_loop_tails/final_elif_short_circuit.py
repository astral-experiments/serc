def direct_and(items, first, second):
    for item in items:
        if item == 0:
            first()
        elif item and second:
            second()


def direct_not_or(items, first, second):
    while items:
        item = items.pop()
        if item == 0:
            first()
        elif not (item or second):
            second()


async def direct_async(items, first, second):
    async for item in items:
        if item == 0:
            first()
        elif item and second:
            second()


def nested_loop(items, first, second):
    for item in items:
        if item:
            for nested in item:
                if first:
                    first()
                elif nested and second:
                    second()
        elif second:
            second()
