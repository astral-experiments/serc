def specialized_none_raise(manager, cache, key, value):
    with manager:
        if value is None and cache[key] is not None:
            raise ValueError('already set')
    cache[key] = value

def reversed_none_raise(manager, cache, key, value):
    with manager:
        if cache[key] is not None and value is None:
            raise ValueError('already set')
    cache[key] = value

def equality_raise_control(manager, cache, key, value):
    with manager:
        if value == 0 and cache[key] != 0:
            raise ValueError('already set')
    cache[key] = value

def none_return_control(manager, cache, key, value):
    with manager:
        if value is None and cache[key] is not None:
            return cache[key]
    return value
