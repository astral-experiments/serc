def f(x, obj, flag, other, items):
    if flag:
        if x == 0:
            obj.channel = 0
        else:
            assert False, "unknown"
    elif other:
        obj.x = 42
    obj.result = 3
