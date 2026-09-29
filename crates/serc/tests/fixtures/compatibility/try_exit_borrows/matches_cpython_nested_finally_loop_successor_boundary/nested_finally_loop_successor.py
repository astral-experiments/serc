def f(flag, x, y, values):
    try:
        if flag:
            consume(x)
        else:
            consume(y)
    except Exception:
        pass
    finally:
        try:
            consume(x)
        except Exception:
            pass
        if flag:
            for item in values:
                if flag:
                    consume(item)
        consume(x)
    consume(x, y)
