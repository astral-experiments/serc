def poll(value, limit):
    while ready():
        try:
            try:
                if 0 <= value < limit:
                    return
            except ValueError:
                recover()
        except Exception:
            retry()
        advance()
