async def pass_body(items):
    async for item in items:
        pass

async def two_passes(items):
    async for item in items:
        pass
        pass

async def docstring_and_pass(items):
    async for item in items:
        'docstring'
        pass

async def nested(items):
    async for outer in items:
        async for inner in outer:
            pass

async def with_else(items):
    async for item in items:
        pass
    else:
        pass
