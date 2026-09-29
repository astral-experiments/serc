
def choose(manager, record, present, absent):
    with manager:
        result = present if record.value is not None else absent
    return result
