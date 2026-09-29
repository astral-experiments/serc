def f(flag, other, obj):
    try:
        yield
    except Error as exc:
        if flag:
            raise exc
        assert flag
        assert other
        assert obj.ready
