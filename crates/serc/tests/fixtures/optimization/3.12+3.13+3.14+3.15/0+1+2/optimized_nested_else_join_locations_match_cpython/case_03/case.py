def f(x, y, z, obj, flag, items, manager):
    obj.before = 0
    if flag:
        if x < y < z and flag:
            try:
                obj.channel = 1
            finally:
                obj.done = True
        else:
            assert y
    elif y:
        obj.other = 2
    obj.after = 3
