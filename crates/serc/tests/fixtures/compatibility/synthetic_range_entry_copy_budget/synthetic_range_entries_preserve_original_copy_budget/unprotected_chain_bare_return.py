def poll(value, limit):
    while ready():
        if 0 <= value < limit:
            return
        advance()
