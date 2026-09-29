
def choose(manager, value, present, absent):
    with manager:
        result = absent if value is None else present
    return result
