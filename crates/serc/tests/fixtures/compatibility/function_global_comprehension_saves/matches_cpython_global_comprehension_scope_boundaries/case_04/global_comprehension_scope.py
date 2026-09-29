def f[T](values):
    global saved
    [saved := x for x in values]
    return saved
