async def f(values, selected, tail):
    items = [item for item in values]
    candidate = await fetch(items)
    return pack(items=items, chosen=candidate if selected else None, tail=tail)
