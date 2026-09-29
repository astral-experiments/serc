async def f(values):
    selected = sorted({(left, right) for left in values for right in values if right})
    try:
        result = await consume(selected) if selected else {}
    except Exception:
        result = {}
    return result
