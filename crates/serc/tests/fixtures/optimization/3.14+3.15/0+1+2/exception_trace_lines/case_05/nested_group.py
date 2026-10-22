def run():
    try:
        raise ValueError
    except* ValueError:
        try:
            raise TypeError
        except* TypeError:
            pass
    return 0
