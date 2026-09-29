def outer():
    captured = None
    def f(flag, x, y, obj):
        if flag:
            if x:
                obj.value = 1
            else:
                nonlocal captured
                assert x
        elif y:
            obj.value = 2
        obj.done = True
    return f
