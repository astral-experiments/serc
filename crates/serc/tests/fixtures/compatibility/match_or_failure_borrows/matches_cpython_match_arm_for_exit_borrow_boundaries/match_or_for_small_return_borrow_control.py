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
    return values
