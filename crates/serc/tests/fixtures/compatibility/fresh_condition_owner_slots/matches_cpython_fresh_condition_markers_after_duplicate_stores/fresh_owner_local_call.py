def f(sock, work):
    try:
        readable, _, _ = work()
        if not readable:
            return False
        return False
    except OSError:
        return True
