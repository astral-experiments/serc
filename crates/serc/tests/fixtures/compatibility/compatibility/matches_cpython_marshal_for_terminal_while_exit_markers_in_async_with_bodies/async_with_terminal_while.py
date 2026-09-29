async def direct_name(manager, flag):
    async with manager:
        while flag:
            step()
    outside()

async def direct_attribute(manager, resource):
    async with manager:
        while resource.enabled:
            step()
    outside()

async def direct_call(manager, resource):
    async with manager:
        while resource.enabled():
            step()
    outside()

async def multi_edge(manager, first, second):
    async with manager:
        while first and second:
            step()
    outside()

async def sync_then_async_with(sync_manager, async_manager, flag):
    with sync_manager:
        async with async_manager:
            while flag:
                step()
    outside()

async def async_then_sync_with(async_manager, sync_manager, flag):
    async with async_manager:
        with sync_manager:
            while flag:
                step()
    outside()

async def async_with_inner_finally(manager, flag):
    async with manager:
        try:
            while flag:
                step()
        finally:
            cleanup()
    outside()

async def async_with_inner_except(manager, flag):
    async with manager:
        try:
            while flag:
                step()
        except Error:
            cleanup()
    outside()
