
def choose(manager, value, other, present, absent):
    with manager:
        result = present if value is not None and other else absent
    return result
