
def choose(manager, value, present, absent):
    with manager:
        result = present if value == None else absent
    return result
