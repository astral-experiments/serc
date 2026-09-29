def concatenate(flag, value, other):
    return (value if flag else "") + other

def identical(flag, value, other):
    return (value if flag else None) is other

def positional(flag, value, other):
    return consume(value if flag else None, other)
