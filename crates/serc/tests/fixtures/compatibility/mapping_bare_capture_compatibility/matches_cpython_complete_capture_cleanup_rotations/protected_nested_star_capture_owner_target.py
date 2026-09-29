def f(value, finish):
    try:
        match value:
            case {"outer": {"inner": [*items]}}:
                if items is None:
                    return None
                return items
        return None
    finally:
        finish()
