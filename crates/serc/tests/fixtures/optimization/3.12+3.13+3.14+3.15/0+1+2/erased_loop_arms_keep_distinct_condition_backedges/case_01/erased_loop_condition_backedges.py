def f(items, obj, enabled, active, alternate, left, right, offset, work):
    for item in items:
        value = item + offset
        if obj.flag or enabled:
            assert value
            if item is not None:
                assert value
                assert enabled
        else:
            assert value
            if item is not None:
                assert value
                assert enabled
        if active and (item is left or item is right):
            assert value
            assert enabled
        elif alternate and (item is left or item is right):
            assert value
            assert enabled
            assert active
            assert alternate
        else:
            assert value
            assert enabled
