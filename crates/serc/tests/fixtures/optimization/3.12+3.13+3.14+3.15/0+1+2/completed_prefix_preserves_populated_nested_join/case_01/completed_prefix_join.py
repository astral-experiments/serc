def f(obj, a, b, c, d, work):
    if obj is not None:
        assert obj.ready
        before = obj
        if a:
            first = work()
        else:
            first = work()
        between = obj.ready
        assert a
        if b:
            second = work()
        chosen = c if d else a
        assert b
        after_choice = obj.ready
        if c:
            result = None
        else:
            result = work()
    else:
        assert obj
    after = obj.after
    return after
