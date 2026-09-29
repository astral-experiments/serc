def f(values, candidate, selected, tail):
    items = [item.copy() for item in values]
    return pack(items, chosen=candidate if selected else None, tail=tail)
