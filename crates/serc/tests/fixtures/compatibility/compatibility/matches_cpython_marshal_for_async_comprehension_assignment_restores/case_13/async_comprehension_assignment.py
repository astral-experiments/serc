async def f(chunks, obj):
    obj.value = {chunk async for chunk in chunks}
    return obj
