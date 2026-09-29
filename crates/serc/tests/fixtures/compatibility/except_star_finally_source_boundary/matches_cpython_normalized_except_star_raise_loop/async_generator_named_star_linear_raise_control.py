async def target(manager, leaves):
    try:
        async with manager() as value:
            yield value
    except* Exception as group:
        leaf = leaves(group)
        raise leaf from group
