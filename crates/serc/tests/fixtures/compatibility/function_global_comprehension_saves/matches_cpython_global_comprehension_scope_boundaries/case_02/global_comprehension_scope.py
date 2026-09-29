def f(values):
    global saved
    lambda: [saved := x for x in values]
    return saved
