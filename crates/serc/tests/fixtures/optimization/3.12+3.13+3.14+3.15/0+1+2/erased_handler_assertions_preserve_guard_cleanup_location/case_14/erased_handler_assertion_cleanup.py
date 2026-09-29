def f(flag, other, obj):
    for item in obj.items:
        try:
            yield
        except Error as exc:
            if flag:
                raise exc
            assert flag
            assert other
            assert obj.ready
        else:
            if not flag:
                assert False
