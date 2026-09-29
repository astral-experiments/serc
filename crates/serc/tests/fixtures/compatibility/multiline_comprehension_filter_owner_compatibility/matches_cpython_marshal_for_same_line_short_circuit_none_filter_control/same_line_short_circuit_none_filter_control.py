async def f(load):
    rows = await load()
    return sorted({(row.primary or str(row.fallback)) for row in rows if (row.primary or row.fallback) is not None})
