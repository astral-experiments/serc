async def target(manager, leaves, error_type):
    try:
        async with manager() as value:
            yield value
    except* Exception as group:
        for leaf in leaves(group):
            if isinstance(leaf, error_type):
                raise leaf from group
        raise
