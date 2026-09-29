def collect(value):
    match value:
        case {"outer": {"items": [*items]}} | {"items": [*items]}:
            values = []
            for item in items:
                match item:
                    case {"metadata": {"value": str(found)}}:
                        values.append(found)
        case _:
            return None
    unique = set(values)
    if len(unique) != 1:
        return None
    found = next(iter(unique))
    if found not in ALLOWED:
        return None
    return found
