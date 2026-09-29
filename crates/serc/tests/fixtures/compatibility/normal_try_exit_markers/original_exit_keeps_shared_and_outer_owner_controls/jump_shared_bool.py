def f(flag, call, args):
    try:
        if flag and args:
            return call(*args)
    except AttributeError:
        pass
    if call:
        return call(args)
    return None
