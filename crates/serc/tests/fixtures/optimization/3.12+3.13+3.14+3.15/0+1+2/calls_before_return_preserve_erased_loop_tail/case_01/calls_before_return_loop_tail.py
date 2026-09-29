def f(items, blocked, enabled, work, obj):
    for item in items:
        if item not in blocked:
            if enabled:
                work()
                return work()
            assert item
    return work()
