
def choose(manager, value, present, absent):
    with manager:
        result = present if None is not value else absent
    return result
