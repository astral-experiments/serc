def f(sock):
    try:
        readable, a, b = select.select([sock], [], [], 0)
        if not readable:
            return False
        return False
    except OSError:
        return True
