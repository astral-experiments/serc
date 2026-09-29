async def f(items, flag, other):
    if flag:
        result = [item for item in items]
    else:
        if other:
            work()
        else:
            more()
        assert flag
    assert flag
