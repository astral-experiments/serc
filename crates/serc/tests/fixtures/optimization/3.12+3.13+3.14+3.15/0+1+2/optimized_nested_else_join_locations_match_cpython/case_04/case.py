def f(x, y, z, obj, flag, items, manager):
    obj.before = 0
    if flag:
        if probe(x) and (y or z):
            obj.channel = 1
        else:
            assert y
    elif y:
        obj.other = 2
    obj.after = 3
