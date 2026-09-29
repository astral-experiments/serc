def f(sock):
    try:
        readable, _, _ = select.value
        if not readable:
            return False
        return False
    except OSError:
        return True
