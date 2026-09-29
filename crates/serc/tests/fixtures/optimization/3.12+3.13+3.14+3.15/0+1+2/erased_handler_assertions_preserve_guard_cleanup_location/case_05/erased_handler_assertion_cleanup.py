async def f(flag, other, obj):
    try:
        await obj.work()
    except Error as exc:
        if flag:
            raise exc
        assert flag
        assert other
        assert obj.ready
    else:
        if not flag:
            assert False
