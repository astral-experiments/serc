async def sync_with_direct(manager, items):
    with manager:
        async for item in items:
            consume(item)

async def async_with_direct(manager, items):
    async with manager:
        async for item in items:
            consume(item)

async def sync_then_async_with(sync_manager, async_manager, items):
    with sync_manager:
        async with async_manager:
            async for item in items:
                consume(item)

async def async_then_sync_with(async_manager, sync_manager, items):
    async with async_manager:
        with sync_manager:
            async for item in items:
                consume(item)

async def async_with_two_items(first, second, items):
    async with first, second:
        async for item in items:
            consume(item)

async def async_with_inner_finally(manager, items):
    async with manager:
        try:
            async for item in items:
                consume(item)
        finally:
            cleanup()

async def async_with_nonfinal(manager, items):
    async with manager:
        async for item in items:
            consume(item)
        after()
