def direct(manager):
    with manager:
        try:
            body()
        finally:
            cleanup()

def followed(manager):
    with manager:
        try:
            body()
        finally:
            cleanup()
    after()

def leading_statement(manager):
    with manager:
        before()
        try:
            body()
        finally:
            cleanup()
    after()

def multiple_items(outer, inner):
    with outer, inner:
        try:
            body()
        finally:
            cleanup()
    after()

def enclosing_try(manager):
    try:
        with manager:
            try:
                body()
            finally:
                cleanup()
        after()
    except Exception:
        recover()

def multiple_finalbody_statements(manager):
    with manager:
        try:
            body()
        finally:
            cleanup_one()
            cleanup_two()
    after()

def nested_managers(outer, inner):
    with outer:
        with inner:
            try:
                body()
            finally:
                cleanup()
    after()

async def synchronous_inside_async(async_manager, sync_manager):
    async with async_manager:
        with sync_manager:
            try:
                await body()
            finally:
                await cleanup()
    await after()

def nontrailing():
    with manager:
        try:
            body()
        finally:
            cleanup()
        inside_after()
    outside_after()

def nested_if(manager, flag):
    with manager:
        if flag:
            try:
                body()
            finally:
                cleanup()
    after()

def nested_loop(manager, items):
    with manager:
        for item in items:
            try:
                body(item)
            finally:
                cleanup(item)
    after()

def unreachable_trailing_try(manager):
    with manager:
        return value()
        try:
            body()
        finally:
            cleanup()

async def asynchronous_manager(manager):
    async with manager:
        try:
            await body()
        finally:
            await cleanup()
    await after()

async def synchronous_outside_async(sync_manager, async_manager):
    with sync_manager:
        async with async_manager:
            try:
                await body()
            finally:
                await cleanup()
    await after()
