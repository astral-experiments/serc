
def choose(manager, value, other, present, absent):
    with manager:
        result = present if value is not None is not other else absent
    return result
