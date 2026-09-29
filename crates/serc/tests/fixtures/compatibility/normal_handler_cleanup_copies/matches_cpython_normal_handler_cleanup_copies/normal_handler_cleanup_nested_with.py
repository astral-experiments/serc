def probe(manager, work, a, b, flag, inner):
    with manager:
        try:
            work()
        except ValueError:
            value = 1
            if flag:
                value = 2
                if inner:
                    a()
                else:
                    b()
