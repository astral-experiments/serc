def direct():
    try:
        value = produce()
    except Exception:
        raise RuntimeError
    return value


def nested():
    try:
        try:
            value = produce()
        except SyntaxError:
            raise SyntaxError
        return value
    except Exception:
        raise
