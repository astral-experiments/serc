
def choose(manager, value, present, absent):
    with manager:
        result = present if value is not None else absent
    return result
