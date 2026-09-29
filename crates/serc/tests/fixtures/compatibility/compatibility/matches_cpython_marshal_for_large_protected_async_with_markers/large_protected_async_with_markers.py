async def large_prefix_except(manager, pause, work):
    work(); work(); work(); work(); work(); work(); work(); work()
    work(); work(); work(); work(); work(); work(); work(); work()
    work(); work(); work(); work(); work(); work(); work(); work()
    work(); work(); work(); work(); work(); work(); work(); work()
    work(); work(); work(); work(); work(); work(); work(); work()
    work(); work(); work(); work(); work(); work(); work(); work()
    try:
        async with manager:
            await pause()
    except Error:
        work()


async def large_body_except_else(manager, pause, work):
    try:
        async with manager:
            work(); work(); work(); work(); work(); work(); work(); work()
            work(); work(); work(); work(); work(); work(); work(); work()
            work(); work(); work(); work(); work(); work(); work(); work()
            work(); work(); work(); work(); work(); work(); work(); work()
            work(); work(); work(); work(); work(); work(); work(); work()
            work(); work(); work(); work(); work(); work(); work(); work()
            await pause()
    except Error:
        work()
    else:
        work()


async def large_body_finally(manager, pause, work):
    try:
        async with manager:
            work(); work(); work(); work(); work(); work(); work(); work()
            work(); work(); work(); work(); work(); work(); work(); work()
            work(); work(); work(); work(); work(); work(); work(); work()
            work(); work(); work(); work(); work(); work(); work(); work()
            work(); work(); work(); work(); work(); work(); work(); work()
            work(); work(); work(); work(); work(); work(); work(); work()
            await pause()
    finally:
        work()


async def tuple_assignment_pair(manager, pause, work):
    try:
        async with manager:
            first = work()
            second = await pause()
    except (Error, OtherError):
        work()
    return None


async def tuple_assignment_pair_large_prefix(manager, pause, work):
    work(); work(); work(); work(); work(); work(); work(); work()
    work(); work(); work(); work(); work(); work(); work(); work()
    work(); work(); work(); work(); work(); work(); work(); work()
    work(); work(); work(); work(); work(); work(); work(); work()
    work(); work(); work(); work(); work(); work(); work(); work()
    work(); work(); work(); work(); work(); work(); work(); work()
    try:
        async with manager:
            first = work()
            second = await pause()
    except (Error, OtherError):
        work()
    return None


async def nested_async_with(manager, nested, pause, work):
    work(); work(); work(); work(); work(); work(); work(); work()
    work(); work(); work(); work(); work(); work(); work(); work()
    work(); work(); work(); work(); work(); work(); work(); work()
    work(); work(); work(); work(); work(); work(); work(); work()
    work(); work(); work(); work(); work(); work(); work(); work()
    work(); work(); work(); work(); work(); work(); work(); work()
    try:
        async with manager:
            async with nested:
                await pause()
    except Error:
        work()


async def tuple_nonpair_control(manager, pause, work):
    try:
        async with manager:
            await pause()
    except (Error, OtherError):
        work()
    return None
