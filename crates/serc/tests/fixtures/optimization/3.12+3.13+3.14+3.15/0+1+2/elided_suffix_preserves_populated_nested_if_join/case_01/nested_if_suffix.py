def f(outer, inner, value, obj):
    if outer is None:
        if inner:
            result = 1
        else:
            result = value + 1
        assert inner
    else:
        assert outer
    after = obj.attribute
    return after
