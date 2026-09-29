def f(flag, other, obj):
    try:
        yield
    except Error:
        if flag:
            raise Error
        assert flag
        assert other
        assert obj.ready
    else:
        if not flag:
            assert False
