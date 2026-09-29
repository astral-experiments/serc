def outer(values):
    saved = 0
    def f():
        nonlocal saved
        [saved := x for x in values]
        return saved
