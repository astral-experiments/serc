async def f(chunks, obj):
    try:
        obj.value = [chunk async for chunk in chunks]
    finally:
        await obj.close()
    return obj
