def f(a, b, c, d):
    if a:
        if b:
            raise ValueError
        else:
            if c and d:
                raise RuntimeError
    else:
        sink(1)
        sink(2)
    finish()
