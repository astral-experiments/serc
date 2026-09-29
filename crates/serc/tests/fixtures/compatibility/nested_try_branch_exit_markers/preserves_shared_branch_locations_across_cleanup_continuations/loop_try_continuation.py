def f(a, b, c, manager):
    while again():
        try:
            if a:
                if b:
                    sink(1)
                else:
                    if c:
                        sink(2)
            else:
                sink(3)
                sink(4)
        except Exception:
            sink(6)
        sink(5)
