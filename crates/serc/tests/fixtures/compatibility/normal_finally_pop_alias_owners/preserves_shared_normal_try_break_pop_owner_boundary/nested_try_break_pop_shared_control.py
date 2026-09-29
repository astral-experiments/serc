def nested_try_break_pop_owner(read, ready, cancel, close, finish):
    try:
        try:
            while True:
                if cancel is not None and cancel():
                    finish()
                    return None
                if ready():
                    value = read()
                    break
        except EOFError as error:
            value = None
    finally:
        close()
        finish()
    return value
