def is_none(value):
    return not (value is None)

def is_not_none(value):
    return not (value is not None)

def multiline(value):
    return not (
        value is
        None
    )

def protected(value):
    try:
        return not (value is None)
    except Error:
        return False

def assertion(value):
    assert not (value is None)

def direct_condition(value):
    if value is None:
        return 1
    return 2
