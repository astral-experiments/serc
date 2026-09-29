def f(x, y, obj):
    obj.a = [v for v in x] if y else []
    assert x
