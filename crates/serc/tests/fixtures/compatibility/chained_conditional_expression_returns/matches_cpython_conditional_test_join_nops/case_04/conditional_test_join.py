def f(value, other, flag):
    if (
        value.total == 0
        if flag
        else other.total == 0
    ):
        return False
    return value.done()
