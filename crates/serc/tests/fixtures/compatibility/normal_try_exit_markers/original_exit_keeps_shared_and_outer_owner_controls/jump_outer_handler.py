def f(flag, call, args):
    try:
        try:
            if flag:
                return call(*args)
        except AttributeError:
            pass
        if call:
            return call(args)
        return None
    except Exception:
        report()
