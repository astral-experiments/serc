def f(a, b, c, d, stop):
    if a:
        if b:
            sink(0)
        else:
            if c:
                sink(1)
            else:
                if d:
                    sink(2)
                elif stop:
                    return
    else:
        sink(3)
        sink(4)
    finish()
