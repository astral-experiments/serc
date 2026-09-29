
def choose(manager, first, second, present, absent):
    with manager:
        left = present if first is not None else absent
        right = absent if second is None else present
    return left, right
