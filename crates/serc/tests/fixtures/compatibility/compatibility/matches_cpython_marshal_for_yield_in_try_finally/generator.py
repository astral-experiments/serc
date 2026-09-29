def generator(flag):
    if flag:
        try:
            yield
        finally:
            pass
    else:
        yield

def terminating_try_except_finally():
    try:
        raise StopIteration
    except ValueError:
        yield 1
    finally:
        pass
