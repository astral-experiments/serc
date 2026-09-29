def direct(first, second, third):
    if first <= second <= third:
        return
    after()


def copied_global_exit(first, second, third):
    if first <= second <= third:
        return
    global target
    target = first


def shared_nested_exit(kind, first, second, third):
    if kind == "chain":
        if first <= second <= third:
            return
        global nested_target
        nested_target = first
    else:
        consume()


def identity_chain(xre, xim):
    if xre is xim is None:
        return (None,) * 4
    if xim:
        raise NotImplementedError
    return xre
