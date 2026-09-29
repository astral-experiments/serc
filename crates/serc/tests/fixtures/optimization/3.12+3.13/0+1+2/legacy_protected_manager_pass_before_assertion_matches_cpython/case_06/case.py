def outer():
    outer_marker = 0
    async def f(outer, inner, obj, predicate, cleanup, flag):
        try:
            async with outer:
                with inner:
                    pass
                    assert predicate()
        finally:
            cleanup()
        return
    return f
