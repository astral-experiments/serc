def example(flag, choice, build):
    if flag:
        if choice:
            value = build() if flag else choice
            assert value
        else:
            assert choice
            value = build()
        assert flag
    else:
        return build()
