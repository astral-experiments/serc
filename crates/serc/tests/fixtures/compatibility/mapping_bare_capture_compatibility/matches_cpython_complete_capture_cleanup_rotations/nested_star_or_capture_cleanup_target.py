def f(value):
    match value:
        case {"outer": {"inner": [*items]}} | {"inner": [*items]}:
            return items
    return None
