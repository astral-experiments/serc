def f(flag, other, obj):
    try:
        yield
    except Error as exc:
        if flag or other:
            raise exc
        assert flag
        assert other
        assert obj.ready
    else:
        if not flag:
            assert False
