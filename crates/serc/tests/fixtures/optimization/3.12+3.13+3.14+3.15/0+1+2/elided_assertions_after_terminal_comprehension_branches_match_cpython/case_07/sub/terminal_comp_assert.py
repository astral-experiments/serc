def f(x, y, z, cm):
    if x:
        a = [i for i in y]
    elif z:
        a = {i for i in y}
    else:
        a = {i:i for i in y}
    assert x
