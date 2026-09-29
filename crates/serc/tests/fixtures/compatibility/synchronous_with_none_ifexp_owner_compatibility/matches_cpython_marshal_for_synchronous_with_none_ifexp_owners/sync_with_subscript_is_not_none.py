
def choose(manager, record, present, absent):
    with manager:
        result = present if record[2] is not None else absent
    return result
