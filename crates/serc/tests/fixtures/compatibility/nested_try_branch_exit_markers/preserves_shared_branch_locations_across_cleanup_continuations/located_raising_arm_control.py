def f(a, b, c, manager):
    try:
        if a:
            if b:
                sink(1)
            else:
                if c:
                    raise RuntimeError
        else:
            sink(3)
            sink(4)
    except Exception:
        sink(6)
    sink(5)
