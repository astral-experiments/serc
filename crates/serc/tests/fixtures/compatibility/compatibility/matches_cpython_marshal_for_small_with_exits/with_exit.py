def normal():
    with manager:
        value = 0
    value = 1

def terminal():
    with manager:
        value = 0
        return 1
    value = 1

def nested_suppression():
    with first, \
        second:
        raise Error
    return result
