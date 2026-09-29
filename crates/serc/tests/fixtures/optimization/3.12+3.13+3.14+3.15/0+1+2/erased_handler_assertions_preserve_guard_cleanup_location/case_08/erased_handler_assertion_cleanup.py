def f(flag, other, obj):
    try:
        yield
    except Error as exc:
        if flag:
            raise exc
        pass
    else:
        if not flag:
            assert False
