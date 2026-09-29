async def check(values, selector, left, right, manager):
    with manager:
        await prepare()
        for value in values:
            if value:
                assert (
                    left == expected
                    if selector is expected
                    else RIGHT
                )
            elif left is None:
                assert right is None
            else:
                consume(left)
