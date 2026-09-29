def f(flag, x, y, obj):
    if flag:
        if x:
            obj.value = 1
        else:
            assert x
            global g
    elif y:
        obj.value = 2
    obj.done = True
