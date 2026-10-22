def example(x, y, z, w):
    try:
        before()
    except Exception:
        raise
    finally:
        if x:
            a()
        elif y:
            if z:
                b()
            elif w:
                c()
            else:
                assert False
        else:
            assert False
    after()
