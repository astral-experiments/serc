def f(x, y, z, obj, flag, items, manager):
    try:
        obj.before = 0
        if flag:
            if not (x < y < z):
                obj.channel = 1
            else:
                pass
        elif y:
            obj.other = 2
        obj.after = 3
    finally:
        obj.cleaned = True
