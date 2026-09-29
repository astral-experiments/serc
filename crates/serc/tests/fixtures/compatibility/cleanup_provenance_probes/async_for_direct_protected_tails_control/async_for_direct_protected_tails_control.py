async def direct_try(items):
    try:
        async for item in items:
            await consume(item)
    finally:
        cleanup()


async def direct_with(items, manager):
    with manager:
        async for item in items:
            await consume(item)


async def direct_handler(items):
    try:
        prepare()
    except RuntimeError:
        async for item in items:
            await consume(item)


async def direct_finally(items):
    try:
        prepare()
    finally:
        async for item in items:
            await consume(item)


async def finally_inside_with(items, manager):
    with manager:
        try:
            prepare()
        finally:
            async for item in items:
                await consume(item)


async def finally_inside_try(items):
    try:
        try:
            prepare()
        finally:
            async for item in items:
                await consume(item)
    except RuntimeError:
        recover()
