def f(flag, enabled, count, inner, obj):
    if not flag:
        assert flag
        assert count
    if enabled and count > 0:
        value = obj.make()
        use(value)
        value = 1.0
        other = 2.0
        use(other)
    else:
        assert count
