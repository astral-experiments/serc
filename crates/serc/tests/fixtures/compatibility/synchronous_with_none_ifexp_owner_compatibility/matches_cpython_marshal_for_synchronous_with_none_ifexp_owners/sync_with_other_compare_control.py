
def choose(manager, value, present, absent):
    with manager:
        result = present if value is not absent else absent
    return result
