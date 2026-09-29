
def choose(manager, value, present, absent):
    with manager:
        yield present if value is not None else absent
