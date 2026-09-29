def outer():
    outer_marker = 0
    async def f(manager, outer, obj, predicate, cleanup):
        try:
            async with outer:
                with manager as value:
                    pass
                    global marker
        finally:
            cleanup()
        return
    return f
