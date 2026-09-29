def f(values):
    global saved
    (saved := x for x in values)
    return saved
