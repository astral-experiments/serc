def f(a, b, c):
    if a:
        if b:
            raise ValueError
        else:
            if c:
                raise RuntimeError
    else:
        sink(1)
        sink(2)
    finish()
