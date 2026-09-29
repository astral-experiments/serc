async def f(values):
    selected = sorted({(lambda: item) for item in values if item})
    try:
        result = await consume(selected) if selected else {}
    except Exception:
        result = {}
    return result
