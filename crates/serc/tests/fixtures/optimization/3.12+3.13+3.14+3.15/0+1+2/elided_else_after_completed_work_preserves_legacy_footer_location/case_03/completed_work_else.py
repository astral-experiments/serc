def f(flag, enabled, count, inner, obj):
    if not flag:
        assert flag
        assert count
    if enabled:
        assert enabled
        value = obj.make()
        use(value)
        if inner:
            use(value)
            use(value)
        value = 1.0
        assert value
        other = 2.0
        use(other)
    else:
        assert count
