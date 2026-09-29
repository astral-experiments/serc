
def f(items,a,b,c,d,obj,cm):
    for x in items:
        if c:
            if a or b:
                obj.action()
            else:
                assert a
        else:
            obj.action()
    else:
        obj.action()
