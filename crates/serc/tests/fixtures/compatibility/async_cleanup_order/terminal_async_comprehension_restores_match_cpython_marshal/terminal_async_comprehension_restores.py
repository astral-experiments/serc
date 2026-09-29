async def protected_return(stream, recover):
    try:
        return [item async for item in stream]
    except ValueError:
        recover()

async def protected_sequential_return(first, second, recover):
    try:
        initial = [item async for item in first]
        return [item async for item in second]
    except ValueError:
        recover()

async def protected_awaited_return(stream, use, recover):
    try:
        await use()
        return [item async for item in stream]
    except ValueError:
        recover()

async def protected_nested_return(streams, recover):
    try:
        return [[item async for item in stream] async for stream in streams]
    except ValueError:
        recover()
