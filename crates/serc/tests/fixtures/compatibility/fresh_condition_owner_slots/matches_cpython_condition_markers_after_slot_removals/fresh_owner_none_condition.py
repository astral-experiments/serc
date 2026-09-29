def f(sock):
    try:
        readable, _, _ = select.select([sock], [], [], 0)
        if readable is None:
            return False
        return False
    except OSError:
        return True
