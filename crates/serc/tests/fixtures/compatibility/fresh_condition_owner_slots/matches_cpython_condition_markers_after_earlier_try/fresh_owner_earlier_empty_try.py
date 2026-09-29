def f(sock):
    try:
        pass
    finally:
        pass
    try:
        readable, _, _ = select.select([sock], [], [], 0)
        if not readable:
            return False
        return False
    except OSError:
        return True
