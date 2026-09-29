def f(a, b, c):
    if a:
        if b:
            sink(0)
        else:
            if c:
                sink(3)
    else:
        sink(1)
        sink(2)
    finish()
