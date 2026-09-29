def dynamic(value, other):
    return not (value is other)

def none_on_left(value):
    return not (None is value)

def true_on_right(value):
    return not (value is True)

def false_on_right(value):
    return not (value is False)

def membership(value, values):
    return not (value in values)

def chained(value, other):
    return not (value is None is other)
