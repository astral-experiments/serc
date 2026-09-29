
def reraise():
    raise

def chained(exception, cause):
    message = str(exception)
    raise ValueError(message) from cause

def exhaustive(first, second):
    if first:
        return 1
    elif second:
        raise TypeError(second)
    else:
        return 3

def conjunction(first, second, third):
    if first and second and third:
        return first + third
    return second

def partial_assignment(first, second):
    if first and (local := second):
        return local
    return local

def continuing_assignment(first, second):
    if (local := first) and second:
        return second
    return local
