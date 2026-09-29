async def positional(values, source):
    result = [item async for item in source([value for value in values])]
    return result

async def keyword(values, source):
    result = [item async for item in source(values=[value for value in values])]
    return result

async def dictionary(values, source):
    result = {item: item async for item in source([value for value in values])}
    return result

async def nested_call(values, source, wrap):
    result = [item async for item in source(wrap([value for value in values]))]
    return result

async def conditional(values, source, enabled):
    result = [item async for item in source([value for value in values] if enabled else [])]
    return result
