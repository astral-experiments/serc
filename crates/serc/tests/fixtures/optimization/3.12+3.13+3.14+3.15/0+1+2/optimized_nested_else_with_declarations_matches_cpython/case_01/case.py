def f(flag, x, y, obj):
    if flag:
        if x:
            obj.value = 1
        elif y:
            obj.value = 2
        else:
            global g
            assert x
    elif y:
        obj.value = 3
    obj.done = True
