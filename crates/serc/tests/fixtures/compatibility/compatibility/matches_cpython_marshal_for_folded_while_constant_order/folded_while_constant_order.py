async def empty_pool_retained(action, consume):
    while True:
        await action()
        consume(flag=True)

async def prefixed_unused(action):
    "doc"
    while True:
        await action()

async def prefixed_retained(action, consume):
    "doc"
    while True:
        await action()
        consume(flag=True)

async def prefixed_retained_assignment(action):
    "doc"
    while True:
        await action()
        marker = True

async def await_before_loop(action, consume):
    await action()
    while True:
        await action()
        consume(flag=True)
