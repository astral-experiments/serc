def f(sock):
    try:
        readable, _, _ = sock
        if not readable:
            return False
        return False
    except OSError:
        return True
