def normal_sync(flag, value):
    if flag:
        with manager():
            try:
                return work()
            except Error:
                pass
    return work(value)

async def normal_async_manager(flag, value):
    if flag:
        async with manager():
            try:
                return work()
            except Error:
                pass
    return await work()

async def normal_sync_manager_in_coroutine(flag, value):
    if flag:
        with manager():
            try:
                return work()
            except Error:
                pass
    return await work()

def small_return_control(flag, value):
    if flag:
        with manager():
            try:
                return work()
            except Error:
                pass
    return value

def terminating_handler_control(flag, value):
    if flag:
        with manager():
            try:
                return work()
            except Error:
                return value
    return work(value)
