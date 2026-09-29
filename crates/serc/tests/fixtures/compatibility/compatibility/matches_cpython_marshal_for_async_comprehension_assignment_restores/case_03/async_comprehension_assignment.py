async def f(chunks, out):
    out["value"] = [chunk async for chunk in chunks]
    return out
