def guard(flag, other):
    if flag:
        return flag + 1
    if other is None:
        raise ValueError('missing', other)
    return other

def side_local(flag):
    if flag:
        value = 4
        return value
    return value

def side_deletion(flag):
    if flag:
        del flag
        raise RuntimeError('deleted')
    return flag

def continuing_binding(flag, value):
    prepared = value
    if flag:
        prepared = flag
        return prepared
    if prepared is None:
        return 42
    return prepared

def side_check(flag):
    if flag:
        result
        raise RuntimeError
    return result
