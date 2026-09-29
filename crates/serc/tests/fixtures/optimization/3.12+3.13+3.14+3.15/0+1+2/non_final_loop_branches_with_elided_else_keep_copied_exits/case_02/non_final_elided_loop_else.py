
def f(items,a,b,c,d,obj):
    for x in items:
        if c:
            if a and b:
                obj.action()
            else:
                assert b
        else:
            a=d
